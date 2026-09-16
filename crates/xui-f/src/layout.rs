use std::{collections::HashSet, ops::Range};

use unicode_linebreak::{BreakOpportunity, linebreaks};
use unicode_segmentation::UnicodeSegmentation;
use xui_interface::{
    ComputedTextStyle, FontFamily, FontQuery, FontStretch, FontStyle, FontWeight, GlyphFlags,
    GlyphInstance, GlyphRun, LineHeight, LineLayout, OverflowWrap, ParagraphLayout, Point, Rect,
    TextAlign, TextCluster, TextContent, TextLayoutConstraints, TextLayoutInput, TextOffset,
    TextOverflow, TextRange, WhiteSpace,
};

use crate::{
    FFontId, FGlyphKey, bidi,
    font::{FaceMetrics, FontStore},
    shape::{self, ShapedLine},
};

#[derive(Debug, Clone)]
struct HardParagraph {
    content: Range<usize>,
    hard_break: bool,
    source_line: usize,
}

/// Width-independent state reused while Taffy probes a paragraph with
/// different intrinsic and definite sizes.
#[derive(Debug, Default)]
pub enum FParagraphState {
    #[default]
    Initial,
    HardLayout(FHardLayout),
}

/// An opaque, hard-break-only layout. Soft wrapping and paint positioning are
/// deliberately excluded because they depend on the current width constraint.
#[derive(Debug)]
pub struct FHardLayout {
    key: HardLayoutKey,
    bidi: bidi::Resolver,
    strut: FaceMetrics,
    paragraphs: Vec<PreparedHardParagraph>,
}

#[derive(Debug, Clone)]
struct HardLayoutKey {
    text: TextContent,
    style: ShapeStyleKey,
    font_context_revision: u64,
    font_epoch: u64,
}

impl PartialEq for HardLayoutKey {
    fn eq(&self, other: &Self) -> bool {
        self.text.as_str() == other.text.as_str()
            && self.style == other.style
            && self.font_context_revision == other.font_context_revision
            && self.font_epoch == other.font_epoch
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ShapeStyleKey {
    font_family: FontFamily,
    font_size_bits: u32,
    font_weight: FontWeight,
    font_style: FontStyle,
    letter_spacing_bits: u32,
}

#[derive(Debug)]
struct PreparedHardParagraph {
    source: HardParagraph,
    measured: ShapedLine,
    units: Vec<MeasureUnit>,
    allowed_breaks: HashSet<usize>,
}

#[derive(Debug, Clone)]
struct LineSpec {
    paragraph_index: usize,
    range: Range<usize>,
    paragraph_start: usize,
    source_line: usize,
    hard_break: bool,
    last_in_paragraph: bool,
}

#[derive(Debug, Clone)]
struct MeasureUnit {
    range: Range<usize>,
    advance: f32,
}

pub(crate) fn layout(
    fonts: &mut FontStore,
    state: &mut FParagraphState,
    input: TextLayoutInput,
) -> ParagraphLayout<FFontId, FGlyphKey> {
    let mut output = empty_layout();
    if input.text_box_style.max_lines == Some(0) {
        return output;
    }
    prepare_if_needed(fonts, state, &input);
    let FParagraphState::HardLayout(hard_layout) = state else {
        unreachable!("paragraph state must be prepared before layout")
    };
    let text = input.text.as_str();
    let bidi = &hard_layout.bidi;

    let mut specs = Vec::new();
    for (paragraph_index, paragraph) in hard_layout.paragraphs.iter().enumerate() {
        let ranges = break_paragraph(
            paragraph,
            input.constraints,
            input.paragraph_style.white_space,
            input.paragraph_style.overflow_wrap,
        );
        let range_count = ranges.len();
        for (index, range) in ranges.into_iter().enumerate() {
            specs.push(LineSpec {
                paragraph_index,
                range,
                paragraph_start: paragraph.source.content.start,
                source_line: paragraph.source.source_line,
                hard_break: paragraph.source.hard_break && index + 1 == range_count,
                last_in_paragraph: index + 1 == range_count,
            });
        }
    }

    let max_lines = input.text_box_style.max_lines.unwrap_or(usize::MAX);
    let hidden_lines = specs.len() > max_lines;
    specs.truncate(max_lines);
    let strut = hard_layout.strut;
    let mut pen_y = 0.0;

    for (line_index, spec) in specs.iter().enumerate() {
        let is_last_output = line_index + 1 == specs.len();
        let mut visible_range = spec.range.clone();
        let prepared = &hard_layout.paragraphs[spec.paragraph_index];
        let mut shaped = if visible_range == prepared.source.content {
            prepared.measured.clone()
        } else {
            shape::shape_line(
                fonts,
                text,
                visible_range.clone(),
                &input.default_style,
                bidi,
            )
        };
        let width_limit = constraint_width(input.constraints);
        let nowrap_overflow = is_last_output
            && !wraps(input.paragraph_style.white_space)
            && width_limit.is_some_and(|width| shaped.width > width);
        let ellipsized = is_last_output
            && input.text_box_style.overflow == TextOverflow::Ellipsis
            && (hidden_lines || nowrap_overflow);
        let mut ellipsis = ellipsized.then(|| {
            let anchor = visible_range.end;
            let ellipsis_text = "…";
            let ellipsis_bidi = bidi::Resolver::new(ellipsis_text);
            let mut ellipsis = shape::shape_line(
                fonts,
                ellipsis_text,
                0..ellipsis_text.len(),
                &input.default_style,
                &ellipsis_bidi,
            );
            make_synthetic(&mut ellipsis, anchor, u8::from(shaped.base_rtl));
            ellipsis
        });

        if let Some(ellipsis) = &ellipsis
            && let Some(limit) = width_limit
        {
            while visible_range.start < visible_range.end && shaped.width + ellipsis.width > limit {
                visible_range.end = previous_grapheme_boundary(text, visible_range.clone());
                shaped = shape::shape_line(
                    fonts,
                    text,
                    visible_range.clone(),
                    &input.default_style,
                    bidi,
                );
            }
        }
        if let Some(ellipsis) = &mut ellipsis {
            make_synthetic(ellipsis, visible_range.end, u8::from(shaped.base_rtl));
        }

        let box_metrics = line_box(
            pen_y,
            [Some(&shaped), ellipsis.as_ref()],
            strut,
            &input.default_style,
        );
        pen_y += box_metrics.height;

        emit_line(
            &mut output,
            &shaped,
            ellipsis.as_ref(),
            visible_range,
            spec,
            box_metrics,
            width_limit,
            input.paragraph_style.align,
            input.default_style.font_weight,
            ellipsized,
        );
    }
    output
}

/// Vertical placement of a single line box.
#[derive(Debug, Clone, Copy)]
struct LineBox {
    y: f32,
    height: f32,
    baseline: f32,
}

/// Sizes a line box from the faces it actually uses.
///
/// `LineHeight::Normal` follows the CSS meaning: the box is as tall as the
/// tallest face on the line needs (`ascent + descent + line gap`), so glyphs
/// are never clipped by the paragraph's own bounds. This matters for CJK
/// fallbacks, whose ascent alone can exceed one em. Explicit line heights are
/// honoured verbatim and the extra room (or overflow) is split evenly above and
/// below the glyphs, the same half-leading rule browsers apply.
fn line_box<'a>(
    y: f32,
    pieces: impl IntoIterator<Item = Option<&'a ShapedLine>>,
    strut: FaceMetrics,
    style: &ComputedTextStyle,
) -> LineBox {
    let metrics = pieces
        .into_iter()
        .flatten()
        .flat_map(|piece| piece.runs.iter())
        .fold(strut, |metrics, run| metrics.max(run.metrics));
    let font_size = style.font_size.max(0.0);
    let ascent = metrics.ascent * font_size;
    let descent = metrics.descent * font_size;
    let height = match style.line_height {
        LineHeight::Normal => metrics.line_height() * font_size,
        LineHeight::Px(px) => px,
        LineHeight::Em(em) => em * font_size,
    }
    .max(1.0);
    let half_leading = (height - ascent - descent) * 0.5;
    LineBox {
        y,
        height,
        baseline: y + half_leading + ascent,
    }
}

fn prepare_if_needed(fonts: &mut FontStore, state: &mut FParagraphState, input: &TextLayoutInput) {
    let key = HardLayoutKey::new(input, fonts.epoch());

    if matches!(state, FParagraphState::HardLayout(layout) if layout.key == key) {
        return;
    }

    let text = input.text.as_str();
    let bidi = bidi::Resolver::from_content(input.text.clone());
    let strut = fonts.strut_metrics(&FontQuery {
        families: vec![input.default_style.font_family.clone()],
        weight: input.default_style.font_weight,
        style: input.default_style.font_style,
        stretch: FontStretch::Normal,
    });
    let paragraphs = hard_paragraphs(text)
        .into_iter()
        .map(|source| {
            let measured = shape::shape_line(
                fonts,
                text,
                source.content.clone(),
                &input.default_style,
                &bidi,
            );
            PreparedHardParagraph::new(text, source, measured)
        })
        .collect();
    *state = FParagraphState::HardLayout(FHardLayout {
        key,
        bidi,
        strut,
        paragraphs,
    });
}

impl HardLayoutKey {
    fn new(input: &TextLayoutInput, font_epoch: u64) -> Self {
        Self {
            text: input.text.clone(),
            style: ShapeStyleKey::from(&input.default_style),
            font_context_revision: input.font_context_revision,
            font_epoch,
        }
    }
}

impl From<&ComputedTextStyle> for ShapeStyleKey {
    fn from(style: &ComputedTextStyle) -> Self {
        Self {
            font_family: style.font_family.clone(),
            font_size_bits: style.font_size.to_bits(),
            font_weight: style.font_weight,
            font_style: style.font_style,
            letter_spacing_bits: style.letter_spacing.to_bits(),
        }
    }
}

impl PreparedHardParagraph {
    fn new(text: &str, source: HardParagraph, measured: ShapedLine) -> Self {
        let mut shaped_clusters: Vec<_> = measured
            .runs
            .iter()
            .flat_map(|run| run.clusters.iter())
            .map(|cluster| MeasureUnit {
                range: cluster.text_range.clone(),
                advance: cluster.advance,
            })
            .collect();
        shaped_clusters.sort_by_key(|unit| unit.range.start);
        shaped_clusters.dedup_by(|right, left| {
            if right.range == left.range {
                left.advance = left.advance.max(right.advance);
                true
            } else {
                false
            }
        });

        let mut units: Vec<_> = text[source.content.clone()]
            .grapheme_indices(true)
            .map(|(offset, grapheme)| MeasureUnit {
                range: (source.content.start + offset)
                    ..(source.content.start + offset + grapheme.len()),
                advance: 0.0,
            })
            .collect();
        for cluster in shaped_clusters {
            let covered: Vec<_> = units
                .iter()
                .enumerate()
                .filter(|(_, unit)| {
                    unit.range.start < cluster.range.end && cluster.range.start < unit.range.end
                })
                .map(|(index, _)| index)
                .collect();
            let share = cluster.advance / covered.len().max(1) as f32;
            for index in covered {
                units[index].advance += share;
            }
        }

        let allowed_breaks = linebreaks(&text[source.content.clone()])
            .filter_map(|(offset, opportunity)| {
                matches!(
                    opportunity,
                    BreakOpportunity::Allowed | BreakOpportunity::Mandatory
                )
                .then_some(source.content.start + offset)
            })
            .collect();

        Self {
            source,
            measured,
            units,
            allowed_breaks,
        }
    }
}

fn break_paragraph(
    paragraph: &PreparedHardParagraph,
    constraints: TextLayoutConstraints,
    white_space: WhiteSpace,
    overflow_wrap: OverflowWrap,
) -> Vec<Range<usize>> {
    let range = paragraph.source.content.clone();
    if range.is_empty() {
        return vec![range];
    }
    let Some(max_width) = constraint_width(constraints) else {
        return vec![range];
    };
    if !wraps(white_space) {
        return vec![range];
    }

    let units = &paragraph.units;
    if units.is_empty() {
        return vec![range];
    }

    let allowed = &paragraph.allowed_breaks;
    let mut lines = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let mut width = 0.0;
        let mut cursor = start;
        let mut last_allowed = None;
        let mut committed = None;
        while cursor < units.len() {
            let next_width = width + units[cursor].advance;
            if next_width > max_width && cursor > start {
                committed = match overflow_wrap {
                    OverflowWrap::Anywhere => Some(cursor),
                    OverflowWrap::BreakWord => last_allowed.or(Some(cursor)),
                    OverflowWrap::Normal => last_allowed.or_else(|| {
                        (cursor..units.len())
                            .find(|index| allowed.contains(&units[*index].range.end))
                            .map(|index| index + 1)
                    }),
                };
                if committed.is_some() {
                    break;
                }
            }
            width = next_width;
            cursor += 1;
            if allowed.contains(&units[cursor - 1].range.end) {
                last_allowed = Some(cursor);
            }
            if cursor == units.len() {
                committed = Some(cursor);
            }
        }
        // Even a zero-width box consumes one complete shape cluster. This is
        // the key invariant that prevents short-line wrapping loops.
        let end = committed
            .unwrap_or_else(|| (start + 1).min(units.len()))
            .max(start + 1);
        lines.push(units[start].range.start..units[end - 1].range.end);
        start = end;
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn emit_line(
    output: &mut ParagraphLayout<FFontId, FGlyphKey>,
    shaped: &ShapedLine,
    ellipsis: Option<&ShapedLine>,
    text_range: Range<usize>,
    spec: &LineSpec,
    line_box: LineBox,
    width_limit: Option<f32>,
    align: TextAlign,
    font_weight: xui_interface::FontWeight,
    ellipsized: bool,
) {
    let LineBox {
        y,
        height: line_height,
        baseline,
    } = line_box;
    let mut pieces = Vec::new();
    if shaped.base_rtl {
        if let Some(ellipsis) = ellipsis {
            pieces.push(ellipsis);
        }
        pieces.push(shaped);
    } else {
        pieces.push(shaped);
        if let Some(ellipsis) = ellipsis {
            pieces.push(ellipsis);
        }
    }
    let natural_width: f32 = pieces.iter().map(|piece| piece.width).sum();
    let available = width_limit.unwrap_or(natural_width);
    let base_rtl = shaped.base_rtl;
    let x = match align {
        TextAlign::Center => (available - natural_width).max(0.0) * 0.5,
        TextAlign::Start if base_rtl => (available - natural_width).max(0.0),
        TextAlign::End if !base_rtl => (available - natural_width).max(0.0),
        _ => 0.0,
    };
    let justify = align == TextAlign::Justify && !spec.last_in_paragraph && !ellipsized;
    let justify_until = pieces
        .iter()
        .flat_map(|piece| &piece.runs)
        .flat_map(|run| &run.clusters)
        .filter(|cluster| !cluster.whitespace)
        .map(|cluster| cluster.text_range.end)
        .max()
        .unwrap_or(text_range.start);
    let spaces = pieces
        .iter()
        .flat_map(|piece| &piece.runs)
        .flat_map(|run| &run.clusters)
        .filter(|cluster| cluster.whitespace && cluster.text_range.start < justify_until)
        .count();
    let extra_per_space = if justify && spaces > 0 && available > natural_width {
        (available - natural_width) / spaces as f32
    } else {
        0.0
    };
    let width = natural_width + extra_per_space * spaces as f32;
    let line_run_start = output.runs.len();
    let line_glyph_start = output.glyphs.len();
    let line_cluster_start = output.clusters.len();
    let mut visual_x = 0.0;

    for piece in pieces {
        for run in &piece.runs {
            let run_glyph_start = output.glyphs.len();
            let mut run_cluster_x = 0.0;
            for cluster in &run.clusters {
                let output_cluster = output.clusters.len();
                let cluster_glyph_start = output.glyphs.len();
                let mut glyph_pen = 0.0;
                for glyph in &run.glyphs[cluster.glyph_range.clone()] {
                    output.glyphs.push(GlyphInstance {
                        key: glyph.key,
                        glyph_id: glyph.glyph_id,
                        draw_pos: Point::new(
                            x + visual_x + glyph.offset.x - run_cluster_x,
                            baseline + glyph.offset.y,
                        ),
                        hitbox: Rect::new(
                            x + visual_x + glyph_pen,
                            y,
                            glyph.advance.abs(),
                            line_height,
                        ),
                        cluster: output_cluster,
                        flags: glyph.flags,
                    });
                    glyph_pen += glyph.advance;
                }
                let mapped = if cluster.text_range.start == cluster.text_range.end {
                    text_range.end..text_range.end
                } else {
                    cluster.text_range.clone()
                };
                output.clusters.push(TextCluster {
                    source_line: spec.source_line,
                    local_text_range: mapped.start.saturating_sub(spec.paragraph_start)
                        ..mapped.end.saturating_sub(spec.paragraph_start),
                    text_range: byte_range(mapped),
                    glyph_range: cluster_glyph_start..output.glyphs.len(),
                    hitbox: Rect::new(x + visual_x, y, cluster.advance, line_height),
                });
                visual_x += cluster.advance;
                run_cluster_x += cluster.advance;
                if cluster.whitespace && cluster.text_range.start < justify_until {
                    visual_x += extra_per_space;
                }
            }
            output.runs.push(GlyphRun {
                text_range: byte_range(run.text_range.clone()),
                glyph_range: run_glyph_start..output.glyphs.len(),
                font_id: run.font_id,
                font_size: run
                    .glyphs
                    .first()
                    .map_or(0.0, |glyph| f32::from_bits(glyph.key.font_size_bits)),
                font_weight,
                style_id: 0,
                bidi_level: run.bidi_level,
            });
        }
    }

    output.lines.push(LineLayout {
        source_line: spec.source_line,
        text_range: byte_range(text_range),
        run_range: line_run_start..output.runs.len(),
        glyph_range: line_glyph_start..output.glyphs.len(),
        cluster_range: line_cluster_start..output.clusters.len(),
        x,
        y,
        width,
        height: line_height,
        baseline,
        hard_break: spec.hard_break,
        ellipsized,
    });
}

fn make_synthetic(line: &mut ShapedLine, anchor: usize, bidi_level: u8) {
    for run in &mut line.runs {
        run.text_range = anchor..anchor;
        run.bidi_level = bidi_level;
        for cluster in &mut run.clusters {
            cluster.text_range = anchor..anchor;
        }
        for glyph in &mut run.glyphs {
            glyph.flags |= GlyphFlags::SYNTHETIC;
        }
    }
}

fn previous_grapheme_boundary(text: &str, range: Range<usize>) -> usize {
    text[range.clone()]
        .grapheme_indices(true)
        .map(|(offset, _)| range.start + offset)
        .next_back()
        .unwrap_or(range.start)
}

fn hard_paragraphs(text: &str) -> Vec<HardParagraph> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut source_line = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        let separator_end = match character {
            '\r' => {
                if chars.peek().is_some_and(|(_, next)| *next == '\n') {
                    chars
                        .next()
                        .map_or(index + 1, |(next, ch)| next + ch.len_utf8())
                } else {
                    index + character.len_utf8()
                }
            }
            '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' => index + character.len_utf8(),
            _ => continue,
        };
        output.push(HardParagraph {
            content: start..index,
            hard_break: true,
            source_line,
        });
        start = separator_end;
        source_line += 1;
    }
    output.push(HardParagraph {
        content: start..text.len(),
        hard_break: false,
        source_line,
    });
    output
}

fn wraps(white_space: WhiteSpace) -> bool {
    matches!(white_space, WhiteSpace::Normal | WhiteSpace::PreWrap)
}

fn constraint_width(constraints: TextLayoutConstraints) -> Option<f32> {
    match constraints {
        TextLayoutConstraints::Definate(width) => Some(width.max(0.0)),
        TextLayoutConstraints::Unbound | TextLayoutConstraints::MinSize => None,
    }
}

fn byte_range(range: Range<usize>) -> TextRange {
    TextRange::new(
        TextOffset::byte_offset(range.start),
        TextOffset::byte_offset(range.end),
    )
}

fn empty_layout() -> ParagraphLayout<FFontId, FGlyphKey> {
    ParagraphLayout {
        lines: Vec::new(),
        runs: Vec::new(),
        glyphs: Vec::new(),
        clusters: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xui_interface::{ParagraphStyle, TextBoxStyle, TextStyle};

    fn shaped_units(widths: &[f32]) -> ShapedLine {
        let mut offset = 0;
        let clusters = widths
            .iter()
            .map(|width| {
                let start = offset;
                offset += 1;
                shape::ShapedCluster {
                    text_range: start..offset,
                    glyph_range: 0..0,
                    advance: *width,
                    whitespace: false,
                }
            })
            .collect();
        ShapedLine {
            runs: vec![shape::ShapedRun {
                text_range: 0..offset,
                font_id: FFontId(0),
                bidi_level: 0,
                metrics: FaceMetrics::FALLBACK,
                glyphs: Vec::new(),
                clusters,
                advance: widths.iter().sum(),
            }],
            width: widths.iter().sum(),
            base_rtl: false,
        }
    }

    fn prepared(text: &str, shaped: ShapedLine) -> PreparedHardParagraph {
        PreparedHardParagraph::new(
            text,
            HardParagraph {
                content: 0..text.len(),
                hard_break: false,
                source_line: 0,
            },
            shaped,
        )
    }

    #[test]
    fn zero_width_still_consumes_one_cluster_per_line() {
        let shaped = shaped_units(&[10.0, 10.0, 10.0]);
        let paragraph = prepared("abc", shaped);
        let lines = break_paragraph(
            &paragraph,
            TextLayoutConstraints::max_width(0.0),
            WhiteSpace::Normal,
            OverflowWrap::Anywhere,
        );
        assert_eq!(lines, [0..1, 1..2, 2..3]);
    }

    #[test]
    fn emergency_wrap_can_split_a_ligature_cluster_at_graphemes() {
        let mut shaped = shaped_units(&[30.0]);
        shaped.runs[0].clusters[0].text_range = 0..3;
        shaped.runs[0].text_range = 0..3;
        let paragraph = prepared("ffi", shaped);
        let lines = break_paragraph(
            &paragraph,
            TextLayoutConstraints::max_width(5.0),
            WhiteSpace::Normal,
            OverflowWrap::Anywhere,
        );
        assert_eq!(lines, [0..1, 1..2, 2..3]);
    }

    #[test]
    fn normal_wrap_preserves_long_words_but_break_word_makes_progress() {
        let shaped = shaped_units(&[10.0, 10.0, 10.0]);
        let paragraph = prepared("abc", shaped);
        let normal = break_paragraph(
            &paragraph,
            TextLayoutConstraints::max_width(5.0),
            WhiteSpace::Normal,
            OverflowWrap::Normal,
        );
        let emergency = break_paragraph(
            &paragraph,
            TextLayoutConstraints::max_width(5.0),
            WhiteSpace::Normal,
            OverflowWrap::BreakWord,
        );
        assert_eq!(normal.len(), 1);
        assert_eq!(normal[0], 0..3);
        assert_eq!(emergency, [0..1, 1..2, 2..3]);
    }

    #[test]
    fn hard_breaks_cover_crlf_and_unicode_separators() {
        let paragraphs = hard_paragraphs("a\r\nb\u{2028}c");
        assert_eq!(
            paragraphs
                .iter()
                .map(|item| item.content.clone())
                .collect::<Vec<_>>(),
            [0..1, 3..4, 7..8]
        );
        assert!(paragraphs[0].hard_break);
        assert!(!paragraphs[2].hard_break);
    }

    #[test]
    fn hard_layout_key_ignores_width_but_tracks_shape_inputs() {
        let mut input = TextLayoutInput::new(
            TextContent::from_static("stable shape"),
            TextLayoutConstraints::max_width(100.0),
            ComputedTextStyle::from(TextStyle::default()),
            ParagraphStyle::default(),
            TextBoxStyle::default(),
            7,
        );
        let original = HardLayoutKey::new(&input, 11);

        input.text = TextContent::copy_from("stable shape");
        assert_eq!(original, HardLayoutKey::new(&input, 11));

        input.constraints = TextLayoutConstraints::max_width(20.0);
        input.paragraph_style.overflow_wrap = OverflowWrap::Anywhere;
        input.text_box_style.max_lines = Some(1);
        assert_eq!(original, HardLayoutKey::new(&input, 11));

        input.default_style.font_size += 1.0;
        assert_ne!(original, HardLayoutKey::new(&input, 11));
        input.default_style.font_size -= 1.0;
        input.text = TextContent::from_static("changed shape");
        assert_ne!(original, HardLayoutKey::new(&input, 11));
        input.text = TextContent::from_static("stable shape");
        assert_ne!(original, HardLayoutKey::new(&input, 12));
    }
}
