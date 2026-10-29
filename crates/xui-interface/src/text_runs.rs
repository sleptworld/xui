//! Styles over parts of a paragraph.
//!
//! A paragraph has one default style and, optionally, runs that override it
//! over byte ranges of its text. Runs come in two kinds, kept apart because
//! they invalidate different things:
//!
//! - [`ShapeRun`]s change glyphs and advances: family, size, weight, slant,
//!   letter spacing. They are part of [`TextLayoutInput`](crate::TextLayoutInput)
//!   and of its cache key.
//! - [`PaintRun`]s change only how glyphs are drawn: color and decoration.
//!   They ride on [`TextPaintProps`](crate::TextPaintProps), so recoloring a
//!   paragraph never reshapes it.
//!
//! [`TextRun`] is the form callers write: one list holding both kinds of
//! override, split against the computed style by [`TextRuns::split`].
//!
//! Every list of runs is a [`TextRuns`]: sorted, non-overlapping and free of
//! empty ranges, whatever it was built from. Ranges are byte offsets into the
//! paragraph's text; a boundary inside a character is moved back to that
//! character's start when the runs are applied.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use crate::{Color, ComputedTextStyle, FontFamily, FontStyle, FontWeight, TextDecoration};

/// The style id a glyph run reports for text in the paragraph's default style.
/// Text in shape run `i` reports `i + 1`.
pub const DEFAULT_TEXT_STYLE_ID: u32 = 0;

/// A run's byte range, for the code that keeps every [`TextRuns`] in order.
pub trait RunRange {
    fn range(&self) -> &Range<usize>;
    fn range_mut(&mut self) -> &mut Range<usize>;
}

/// Runs over a paragraph's text, in order and without overlaps.
///
/// Empty for single-style text, which allocates nothing. Clones share the
/// list, and comparing two clones of one list does not walk it.
pub struct TextRuns<T>(Option<Arc<[T]>>);

pub type ShapeRuns = TextRuns<ShapeRun>;
pub type PaintRuns = TextRuns<PaintRun>;

impl<T> TextRuns<T> {
    pub const fn empty() -> Self {
        Self(None)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn as_slice(&self) -> &[T] {
        self.0.as_deref().unwrap_or(&[])
    }

    fn from_sorted(runs: Vec<T>) -> Self {
        if runs.is_empty() {
            Self(None)
        } else {
            Self(Some(runs.into()))
        }
    }
}

impl<T: RunRange> TextRuns<T> {
    /// Sorts `runs` by start and trims each to begin where the one before it
    /// ends, so where runs overlap the earlier one wins. Runs left empty are
    /// dropped.
    pub fn new(runs: impl Into<Vec<T>>) -> Self {
        let mut runs = runs.into();
        runs.sort_by_key(|run| run.range().start);
        let mut end = 0;
        runs.retain_mut(|run| {
            let range = run.range_mut();
            range.start = range.start.max(end);
            if range.start >= range.end {
                return false;
            }
            end = range.end;
            true
        });
        Self::from_sorted(runs)
    }

    /// The index of the run holding byte `offset`, if any does.
    pub fn index_at(&self, offset: usize) -> Option<usize> {
        let runs = self.as_slice();
        let index = runs.partition_point(|run| run.range().end <= offset);
        runs.get(index)
            .is_some_and(|run| run.range().start <= offset)
            .then_some(index)
    }

    /// The run holding byte `offset`, if any does.
    pub fn at(&self, offset: usize) -> Option<&T> {
        self.index_at(offset).map(|index| &self.as_slice()[index])
    }

    /// Cuts `range` of `text` where runs start and end. Each piece comes with
    /// the index of the run it lies in, or `None` where no run covers it. The
    /// pieces tile `range` in order, and every boundary is a character
    /// boundary of `text`.
    pub fn segments<'a>(&'a self, text: &'a str, range: Range<usize>) -> RunSegments<'a, T> {
        let end = floor_char_boundary(text, range.end);
        let start = floor_char_boundary(text, range.start).min(end);
        let runs = self.as_slice();
        RunSegments {
            text,
            runs,
            index: runs.partition_point(|run| run.range().end <= start),
            pos: start,
            end,
        }
    }
}

impl<T: RunRange> From<Vec<T>> for TextRuns<T> {
    fn from(runs: Vec<T>) -> Self {
        Self::new(runs)
    }
}

impl<T: RunRange, const N: usize> From<[T; N]> for TextRuns<T> {
    fn from(runs: [T; N]) -> Self {
        Self::new(runs)
    }
}

impl<T> Default for TextRuns<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T> Clone for TextRuns<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: fmt::Debug> fmt::Debug for TextRuns<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl<T: PartialEq> PartialEq for TextRuns<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right) || left[..] == right[..],
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: Hash> Hash for TextRuns<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

/// See [`TextRuns::segments`].
pub struct RunSegments<'a, T> {
    text: &'a str,
    runs: &'a [T],
    index: usize,
    pos: usize,
    end: usize,
}

impl<T: RunRange> Iterator for RunSegments<'_, T> {
    type Item = (Range<usize>, Option<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.end {
            return None;
        }
        let floor = |offset| floor_char_boundary(self.text, offset);
        while self
            .runs
            .get(self.index)
            .is_some_and(|run| floor(run.range().end) <= self.pos)
        {
            self.index += 1;
        }
        let start = self.pos;
        let (end, run) = match self.runs.get(self.index) {
            Some(run) if floor(run.range().start) <= start => {
                (floor(run.range().end).min(self.end), Some(self.index))
            }
            Some(run) => (floor(run.range().start).min(self.end), None),
            None => (self.end, None),
        };
        self.pos = end;
        Some((start..end, run))
    }
}

fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The part of a text style that decides which glyphs are drawn and where.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeStyle {
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub letter_spacing: f32,
}

impl ShapeStyle {
    pub fn of(style: &ComputedTextStyle) -> Self {
        Self {
            font_family: style.font_family.clone(),
            font_size: style.font_size,
            font_weight: style.font_weight,
            font_style: style.font_style,
            letter_spacing: style.letter_spacing,
        }
    }

    /// `base` with this style's font settings, and its own everything else.
    pub fn apply(&self, base: &ComputedTextStyle) -> ComputedTextStyle {
        ComputedTextStyle {
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            font_weight: self.font_weight,
            font_style: self.font_style,
            letter_spacing: self.letter_spacing,
            ..base.clone()
        }
    }
}

impl Hash for ShapeStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.font_family.hash(state);
        self.font_size.to_bits().hash(state);
        self.font_weight.hash(state);
        self.font_style.hash(state);
        self.letter_spacing.to_bits().hash(state);
    }
}

/// A stretch of a paragraph shaped with its own font settings.
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct ShapeRun {
    pub range: Range<usize>,
    pub style: ShapeStyle,
}

impl RunRange for ShapeRun {
    fn range(&self) -> &Range<usize> {
        &self.range
    }

    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

/// A stretch of a paragraph drawn in its own color and decoration.
///
/// Paint applies per cluster: a ligature or combining sequence across a run
/// boundary takes the paint of the cluster's first byte.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintRun {
    pub range: Range<usize>,
    pub color: Color,
    pub decoration: TextDecoration,
}

impl RunRange for PaintRun {
    fn range(&self) -> &Range<usize> {
        &self.range
    }

    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

impl Hash for PaintRun {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.range.hash(state);
        for channel in [self.color.r, self.color.g, self.color.b, self.color.a] {
            channel.to_bits().hash(state);
        }
        self.decoration.hash(state);
    }
}

/// What a [`TextRun`] overrides of the computed text style. `None` keeps the
/// computed value.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextRunStyle {
    pub color: Option<Color>,
    pub font_family: Option<FontFamily>,
    pub font_size: Option<f32>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
    pub letter_spacing: Option<f32>,
    pub decoration: Option<TextDecoration>,
}

impl TextRunStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn font_family(mut self, family: impl Into<FontFamily>) -> Self {
        self.font_family = Some(family.into());
        self
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    pub fn font_weight(mut self, weight: FontWeight) -> Self {
        self.font_weight = Some(weight);
        self
    }

    pub fn font_style(mut self, style: FontStyle) -> Self {
        self.font_style = Some(style);
        self
    }

    pub fn letter_spacing(mut self, spacing: f32) -> Self {
        self.letter_spacing = Some(spacing);
        self
    }

    pub fn decoration(mut self, decoration: TextDecoration) -> Self {
        self.decoration = Some(decoration);
        self
    }

    fn shapes(&self) -> bool {
        self.font_family.is_some()
            || self.font_size.is_some()
            || self.font_weight.is_some()
            || self.font_style.is_some()
            || self.letter_spacing.is_some()
    }

    fn paints(&self) -> bool {
        self.color.is_some() || self.decoration.is_some()
    }
}

/// A styled stretch of a paragraph, as a caller writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub range: Range<usize>,
    pub style: TextRunStyle,
}

impl TextRun {
    pub fn new(range: Range<usize>, style: TextRunStyle) -> Self {
        Self { range, style }
    }
}

impl RunRange for TextRun {
    fn range(&self) -> &Range<usize> {
        &self.range
    }

    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
}

impl TextRuns<TextRun> {
    /// The runs that reshape and the runs that only repaint, resolved against
    /// `base`. A run that overrides nothing `base` does not already have is
    /// left out of both, so it cuts no shaping run.
    pub fn split(&self, base: &ComputedTextStyle) -> (ShapeRuns, PaintRuns) {
        let runs = self.as_slice();
        if runs.is_empty() {
            return (TextRuns::empty(), TextRuns::empty());
        }
        let base_shape = ShapeStyle::of(base);
        let mut shape = Vec::new();
        let mut paint = Vec::new();
        for run in runs {
            let style = &run.style;
            if style.shapes() {
                let resolved = ShapeStyle {
                    font_family: style
                        .font_family
                        .clone()
                        .unwrap_or_else(|| base.font_family.clone()),
                    font_size: style.font_size.unwrap_or(base.font_size),
                    font_weight: style.font_weight.unwrap_or(base.font_weight),
                    font_style: style.font_style.unwrap_or(base.font_style),
                    letter_spacing: style.letter_spacing.unwrap_or(base.letter_spacing),
                };
                if resolved != base_shape {
                    shape.push(ShapeRun {
                        range: run.range.clone(),
                        style: resolved,
                    });
                }
            }
            if style.paints() {
                let color = style.color.unwrap_or(base.color);
                let decoration = style.decoration.unwrap_or(base.decoration);
                if color != base.color || decoration != base.decoration {
                    paint.push(PaintRun {
                        range: run.range.clone(),
                        color,
                        decoration,
                    });
                }
            }
        }
        // `self` is already in order, so its parts are too.
        (TextRuns::from_sorted(shape), TextRuns::from_sorted(paint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(range: Range<usize>) -> PaintRun {
        PaintRun {
            range,
            color: Color::BLACK,
            decoration: TextDecoration::default(),
        }
    }

    fn ranges(runs: &PaintRuns) -> Vec<Range<usize>> {
        runs.as_slice()
            .iter()
            .map(|run| run.range.clone())
            .collect()
    }

    #[test]
    fn new_sorts_trims_overlaps_and_drops_empty_runs() {
        let runs = PaintRuns::new(vec![paint(6..9), paint(0..4), paint(2..7), paint(5..5)]);
        assert_eq!(ranges(&runs), [0..4, 4..7, 7..9]);
        assert!(PaintRuns::new(vec![paint(3..3)]).is_empty());
    }

    #[test]
    fn segments_tile_the_range_with_gaps_in_the_default_style() {
        let runs = PaintRuns::new(vec![paint(2..4), paint(6..8)]);
        let segments: Vec<_> = runs.segments("0123456789", 1..7).collect();
        assert_eq!(
            segments,
            [(1..2, None), (2..4, Some(0)), (4..6, None), (6..7, Some(1))]
        );
        let all: Vec<_> = PaintRuns::empty().segments("abc", 0..3).collect();
        assert_eq!(all, [(0..3, None)]);
    }

    #[test]
    fn segments_snap_boundaries_inside_a_character_back_to_its_start() {
        // "é" is two bytes, 1..3: a run over 2..3 starts at 1, and one ending
        // at 2 ends there too.
        let runs = PaintRuns::new(vec![paint(2..3)]);
        let segments: Vec<_> = runs.segments("aéb", 0..4).collect();
        assert_eq!(segments, [(0..1, None), (1..3, Some(0)), (3..4, None)]);
        let runs = PaintRuns::new(vec![paint(0..2)]);
        let segments: Vec<_> = runs.segments("aéb", 0..4).collect();
        assert_eq!(segments, [(0..1, Some(0)), (1..4, None)]);
    }

    #[test]
    fn index_at_finds_the_run_holding_an_offset() {
        let runs = PaintRuns::new(vec![paint(2..4), paint(6..8)]);
        assert_eq!(runs.index_at(1), None);
        assert_eq!(runs.index_at(2), Some(0));
        assert_eq!(runs.index_at(4), None);
        assert_eq!(runs.index_at(7), Some(1));
    }

    #[test]
    fn split_keeps_shaping_and_paint_overrides_apart() {
        let base = ComputedTextStyle::from(crate::TextStyle::default());
        let runs = TextRuns::new(vec![
            TextRun::new(0..2, TextRunStyle::new().font_weight(FontWeight::Bold)),
            TextRun::new(2..4, TextRunStyle::new().color(Color::WHITE)),
            // Overrides nothing the base does not already have.
            TextRun::new(4..6, TextRunStyle::new().font_size(base.font_size)),
        ]);
        let (shape, paint) = runs.split(&base);
        assert_eq!(shape.len(), 1);
        assert_eq!(shape.as_slice()[0].range, 0..2);
        assert_eq!(shape.as_slice()[0].style.font_weight, FontWeight::Bold);
        assert_eq!(ranges(&paint), [2..4]);
        assert_eq!(paint.as_slice()[0].color, Color::WHITE);
    }
}
