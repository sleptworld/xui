use crate::bidi::BidiDirection;
use crate::doc::{Direction, SpanStyle};
use crate::engine::Engine;
use crate::fontique_library::{FamilyList, Font, FontContext, FontGroupId};
use crate::layout::{
    CLUSTER_CONTINUATION, CLUSTER_DETAILED, CLUSTER_EMPTY, CLUSTER_LAST_CONTINUATION,
    CLUSTER_LIGATURE, CLUSTER_NEWLINE, ClusterData, DetailedClusterData, GLYPH_DETAILED, GlyphData,
    LayoutData, LineData, LineLayoutData, RunData,
};
use crate::line_breaker::BreakLines;
use std::borrow::Borrow;
use std::ops::{Deref, Range};
use std::u32;
use swash::shape::{ShapeContext, Shaper};
use swash::text::analyze;
use swash::text::cluster::{Boundary, CharCluster, ClusterInfo, Parser, Token};
use swash::text::{Language, Script, cluster::CharInfo};
use swash::{GlyphId, NormalizedCoord, Setting, Stretch, Style, Synthesis, Weight};

const MAX_ID: SpanId = SpanId(usize::MAX);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SpanId(usize);

impl SpanId {
    pub fn new(v: impl Into<usize>) -> Self {
        Self(v.into())
    }
}

impl Deref for SpanId {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Default, Clone)]
pub struct Par {
    pub data: LayoutData,
    pub line_data: LineLayoutData,
}

impl Par {
    pub fn dump_clusters(&self) {
        for (i, cluster) in self.line_data.clusters.iter().enumerate() {
            println!("[{}] {} @ {}", i, cluster.0, cluster.1);
        }
    }
    /// Creates a new empty paragraph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the current line state and returns a line breaker
    /// for the paragraph.
    pub fn break_lines<'a>(&'a mut self) -> BreakLines<'a> {
        self.line_data.clear();
        BreakLines::new(&mut self.data, &mut self.line_data)
    }

    /// Returns an iterator over the lines in the paragraph.
    pub fn lines<'a>(&'a self) -> Lines<'a> {
        Lines {
            layout: &self.data,
            line_layout: &self.line_data,
            iter: self.line_data.lines.iter(),
        }
    }

    /// Clears the paragraph.
    pub fn clear(&mut self) {
        self.data.clear();
        self.line_data.clear();
    }
}

impl Par {
    pub(super) fn push_run<'a>(
        &mut self,
        spans: &[SpanData],
        font: Font,
        size: f32,
        level: u8,
        shaper: Shaper<'a>,
    ) {
        let coords_start = self.data.coords.len() as u32;
        self.data
            .coords
            .extend_from_slice(shaper.normalized_coords());
        let coords_end = self.data.coords.len() as u32;
        let mut clusters_start = self.data.clusters.len() as u32;
        let metrics = shaper.metrics();
        let mut advance = 0.;
        let mut last_span = self.data.last_span;
        let mut span_data = &spans[last_span as usize];
        shaper.shape_with(|c| {
            if c.info.boundary() == Boundary::Mandatory {
                self.data
                    .clusters
                    .last_mut()
                    .map(|c| c.flags |= CLUSTER_NEWLINE);
            }
            let span = c.data;
            if span != last_span {
                // Ensure that every run belongs to a single span.
                let clusters_end = self.data.clusters.len() as u32;
                if clusters_end != clusters_start {
                    self.data.runs.push(RunData {
                        span: SpanId(last_span as usize),
                        line: 0,
                        font: font.clone(),
                        coords: (coords_start, coords_end),
                        size,
                        level,
                        whitespace: false,
                        trailing_whitespace: false,
                        clusters: (clusters_start, clusters_end),
                        ascent: metrics.ascent * span_data.line_spacing,
                        descent: metrics.descent * span_data.line_spacing,
                        leading: metrics.leading * span_data.line_spacing,
                        underline: span_data.underline,
                        underline_offset: span_data
                            .underline_offset
                            .unwrap_or(metrics.underline_offset),
                        underline_size: span_data.underline_size.unwrap_or(metrics.stroke_size),
                        strikeout_offset: metrics.strikeout_offset,
                        strikeout_size: metrics.stroke_size,
                        advance,
                    });
                    clusters_start = clusters_end;
                    advance = 0.;
                }
                last_span = span;
                span_data = &spans[last_span as usize];
            }
            let mut glyphs_start = self.data.glyphs.len() as u32;
            let mut cluster_advance = 0.;
            for glyph in c.glyphs {
                cluster_advance += glyph.advance;
                self.push_glyph(glyph);
            }
            advance += cluster_advance;
            let mut component_advance = cluster_advance;
            let is_ligature = c.components.len() > 1;
            let (len, base_flags) = if is_ligature {
                let x = &c.components[0];
                component_advance /= c.components.len() as f32;
                ((x.end - x.start) as u8, CLUSTER_LIGATURE)
            } else {
                ((c.source.end - c.source.start) as u8, 0)
            };
            let glyphs_end = self.data.glyphs.len() as u32;
            if glyphs_end - glyphs_start > 1 || is_ligature {
                let detail_index = self.data.detailed_clusters.len() as u32;
                self.data.detailed_clusters.push(DetailedClusterData {
                    glyphs: (glyphs_start, glyphs_end),
                    advance: component_advance,
                });
                self.data.clusters.push(ClusterData {
                    info: c.info,
                    flags: base_flags | CLUSTER_DETAILED,
                    len,
                    offset: c.source.start,
                    glyphs: detail_index,
                });
            } else {
                let flags = if glyphs_start == glyphs_end {
                    glyphs_start = c.data;
                    CLUSTER_EMPTY
                } else {
                    base_flags
                };
                self.data.clusters.push(ClusterData {
                    info: c.info,
                    flags,
                    len,
                    offset: c.source.start,
                    glyphs: glyphs_start,
                });
            }
            if base_flags != 0 {
                // Emit continuations
                for component in &c.components[1..] {
                    self.data.clusters.push(ClusterData {
                        info: Default::default(),
                        flags: CLUSTER_CONTINUATION | CLUSTER_EMPTY,
                        len: (component.end - component.start) as u8,
                        offset: component.start,
                        glyphs: component_advance.to_bits(),
                    });
                }
                self.data
                    .clusters
                    .last_mut()
                    .map(|c| c.flags |= CLUSTER_LAST_CONTINUATION);
            }
        });
        let clusters_end = self.data.clusters.len() as u32;
        if clusters_end == clusters_start {
            return;
        }
        self.data.last_span = last_span;
        self.data.runs.push(RunData {
            span: SpanId::new(last_span as usize),
            line: 0,
            font,
            coords: (coords_start, coords_end),
            size,
            level,
            whitespace: false,
            trailing_whitespace: false,
            clusters: (clusters_start, clusters_end),
            ascent: metrics.ascent * span_data.line_spacing,
            descent: metrics.descent * span_data.line_spacing,
            leading: metrics.leading * span_data.line_spacing,
            underline: span_data.underline,
            underline_offset: span_data
                .underline_offset
                .unwrap_or(metrics.underline_offset),
            underline_size: span_data.underline_size.unwrap_or(metrics.stroke_size),
            strikeout_offset: metrics.strikeout_offset,
            strikeout_size: metrics.stroke_size,
            advance,
        });
    }

    fn push_glyph(&mut self, glyph: &swash::shape::cluster::Glyph) -> u32 {
        let glyph_index = self.data.glyphs.len() as u32;
        const MAX_SIMPLE_ADVANCE: u32 = 0x7FFF;
        if glyph.x == 0. && glyph.y == 0. {
            let packed_advance = (glyph.advance * 64.) as u32;
            if packed_advance <= MAX_SIMPLE_ADVANCE {
                // Simple glyph
                self.data.glyphs.push(GlyphData {
                    data: glyph.id as u32 | (packed_advance << 16),
                    span: SpanId(glyph.data as usize),
                });
                return glyph_index;
            }
        }
        // Complex glyph
        let detail_index = self.data.detailed_glyphs.len() as u32;
        self.data.detailed_glyphs.push(Glyph::new(glyph));
        self.data.glyphs.push(GlyphData {
            data: GLYPH_DETAILED | detail_index,
            span: SpanId(glyph.data as usize),
        });
        glyph_index
    }

    pub(super) fn apply_spacing(&mut self, spans: &[SpanData]) {
        if spans.len() == 0 {
            return;
        }
        for run in &mut self.data.runs {
            if let Some(span) = spans.get(run.span.0) {
                let word = span.word_spacing;
                let letter = span.letter_spacing;
                if word == 0. && letter == 0. {
                    continue;
                }
                let clusters =
                    &mut self.data.clusters[run.clusters.0 as usize..run.clusters.1 as usize];
                for cluster in clusters {
                    let mut spacing = letter;
                    if word != 0. && cluster.info.whitespace().is_space_or_nbsp() {
                        spacing += word;
                    }
                    if spacing != 0. {
                        let detailed_glyphs = &mut self.data.detailed_glyphs[..];
                        if cluster.is_detailed() && !cluster.is_ligature() {
                            self.data.detailed_clusters[cluster.glyphs as usize].advance += spacing;
                        } else if cluster.is_last_continuation() {
                            cluster.glyphs = (f32::from_bits(cluster.glyphs) + spacing).to_bits();
                        }
                        cluster
                            .glyphs_mut(&self.data.detailed_clusters, &mut self.data.glyphs)
                            .last_mut()
                            .map(|g| {
                                if g.is_simple() {
                                    g.add_spacing(spacing);
                                } else {
                                    detailed_glyphs[g.detail_index()].advance += spacing;
                                }
                                run.advance += spacing;
                            });
                    }
                }
            }
        }
    }

    pub(super) fn finish(&mut self) {
        // Zero out the advance for the extra trailing space that is appended
        // during resolution, and keep the containing run consistent.
        let Some(cluster_index) = self.data.clusters.len().checked_sub(1) else {
            return;
        };
        let Some(cluster) = self.data.clusters.get(cluster_index).copied() else {
            return;
        };
        let advance = cluster.advance(
            &self.data.detailed_clusters,
            &self.data.glyphs,
            &self.data.detailed_glyphs,
        );
        if advance == 0. {
            return;
        }

        if cluster.is_detailed() {
            if let Some(detail) = self.data.detailed_clusters.get_mut(cluster.glyphs as usize) {
                detail.advance = 0.;
                for glyph in self.data.glyphs[make_range(detail.glyphs)].iter_mut() {
                    if glyph.is_simple() {
                        glyph.clear_advance();
                    } else if let Some(glyph) =
                        self.data.detailed_glyphs.get_mut(glyph.detail_index())
                    {
                        glyph.advance = 0.;
                    }
                }
            }
        } else if !cluster.is_empty() {
            if let Some(glyph) = self.data.glyphs.get_mut(cluster.glyphs as usize) {
                if glyph.is_simple() {
                    glyph.clear_advance();
                } else if let Some(glyph) = self.data.detailed_glyphs.get_mut(glyph.detail_index())
                {
                    glyph.advance = 0.;
                }
            }
        }

        if let Some(run) = self.data.runs.iter_mut().rev().find(|run| {
            let cluster_index = cluster_index as u32;
            cluster_index >= run.clusters.0 && cluster_index < run.clusters.1
        }) {
            run.advance = (run.advance - advance).max(0.);
        }
    }
}

/// Sequence of clusters sharing the same font, size and span.
#[derive(Copy, Clone)]
pub struct Run<'a> {
    layout: &'a LayoutData,
    pub(super) run: &'a RunData,
}

impl<'a> Run<'a> {
    pub fn new(layout: &'a LayoutData, run: &'a RunData) -> Self {
        Self { layout, run }
    }
    /// Returns the span that contains the run.
    pub fn span(&self) -> SpanId {
        self.run.span
    }

    /// Returns the font for the run.
    pub fn font(&self) -> &Font {
        &self.run.font
    }

    /// Returns the font size for the run.
    pub fn font_size(&self) -> f32 {
        self.run.size
    }

    /// Returns the bidi level of the run.
    pub fn level(&self) -> u8 {
        self.run.level
    }

    /// Returns the direction of the run.
    pub fn direction(&self) -> swash::shape::Direction {
        if self.run.level & 1 != 0 {
            swash::shape::Direction::RightToLeft
        } else {
            swash::shape::Direction::LeftToRight
        }
    }

    /// Returns the normalized variation coordinates for the run.
    pub fn normalized_coords(&self) -> &'a [NormalizedCoord] {
        self.layout
            .coords
            .get(make_range(self.run.coords))
            .unwrap_or(&[])
    }

    /// Returns the advance of the run.
    pub fn advance(&self) -> f32 {
        self.run.advance
    }

    /// Returns true if the run has an underline decoration.
    pub fn underline(&self) -> bool {
        self.run.underline
    }

    /// Returns the underline offset for the run.
    pub fn underline_offset(&self) -> f32 {
        self.run.underline_offset
    }

    /// Returns the underline size for the run.
    pub fn underline_size(&self) -> f32 {
        self.run.underline_size
    }

    /// Returns an iterator over the clusters in logical order.
    pub fn clusters(&self) -> Clusters<'a> {
        Clusters {
            layout: self.layout,
            iter: self.layout.clusters[make_range(self.run.clusters)].iter(),
            rev: false,
        }
    }

    /// Returns an iterator over the clusters in visual order.
    pub fn visual_clusters(&self) -> Clusters<'a> {
        let rev = self.run.level & 1 != 0;
        Clusters {
            layout: self.layout,
            iter: self.layout.clusters[make_range(self.run.clusters)].iter(),
            rev,
        }
    }
}

/// Iterator over the runs in a paragraph.
#[derive(Clone)]
pub struct Runs<'a> {
    layout: &'a LayoutData,
    iter: core::slice::Iter<'a, RunData>,
}

impl<'a> Iterator for Runs<'a> {
    type Item = Run<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let run = self.iter.next()?;
        Some(Run {
            layout: self.layout,
            run,
        })
    }
}

/// Shaped glyph in a paragraph.
#[derive(Copy, Clone)]
pub struct Glyph {
    /// Glyph identifier.
    pub id: GlyphId,
    /// Horizontal offset.
    pub x: f32,
    /// Vertical offset.
    pub y: f32,
    /// Advance width or height.
    pub advance: f32,
    /// Span that generated the glyph.
    pub span: SpanId,
}

impl Glyph {
    fn new(g: &swash::shape::cluster::Glyph) -> Self {
        Self {
            id: g.id,
            x: g.x,
            y: g.y,
            advance: g.advance,
            span: SpanId(g.data as usize),
        }
    }
}

/// Iterator over a sequence of glyphs in a cluster.
#[derive(Clone)]
pub struct Glyphs<'a> {
    layout: &'a LayoutData,
    iter: core::slice::Iter<'a, GlyphData>,
}

impl<'a> Iterator for Glyphs<'a> {
    type Item = Glyph;

    fn next(&mut self) -> Option<Self::Item> {
        let data = self.iter.next()?;
        if data.is_simple() {
            let (id, advance) = data.simple_data();
            Some(Glyph {
                id,
                x: 0.,
                y: 0.,
                advance,
                span: data.span,
            })
        } else {
            self.layout
                .detailed_glyphs
                .get(data.detail_index())
                .copied()
        }
    }
}

impl<'a> DoubleEndedIterator for Glyphs<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let data = self.iter.next_back()?;
        if data.is_simple() {
            let (id, advance) = data.simple_data();
            Some(Glyph {
                id,
                x: 0.,
                y: 0.,
                advance,
                span: data.span,
            })
        } else {
            self.layout
                .detailed_glyphs
                .get(data.detail_index())
                .copied()
        }
    }
}

/// Collection of glyphs representing an atomic textual unit.
#[derive(Copy, Clone)]
pub struct Cluster<'a> {
    layout: &'a LayoutData,
    cluster: ClusterData,
}

impl<'a> Cluster<'a> {
    pub fn new(layout: &'a LayoutData, cluster: ClusterData) -> Self {
        Self { layout, cluster }
    }

    /// Returns the cluster information.
    pub fn info(&self) -> ClusterInfo {
        self.cluster.info
    }

    /// Returns true if the cluster is empty. This occurs when ignorable
    /// glyphs are removed by the shaper.
    pub fn is_empty(&self) -> bool {
        self.cluster.is_empty()
    }

    /// Returns true if the cluster is a ligature.
    pub fn is_ligature(&self) -> bool {
        self.cluster.is_ligature()
    }

    /// Returns true if the cluster is a continuation of a ligature.
    pub fn is_continuation(&self) -> bool {
        self.cluster.is_continuation()
    }

    /// Returns true if the cluster is the final continuation of a ligature.
    pub fn is_last_continuation(&self) -> bool {
        self.cluster.is_last_continuation()
    }

    /// Returns true if the following cluster is a mandatory line break.
    pub fn is_newline(&self) -> bool {
        self.cluster.is_newline()
    }

    /// Returns the byte offset of the cluster in the source text.
    pub fn offset(&self) -> usize {
        self.cluster.offset as usize
    }

    /// Returns the byte range of the cluster in the source text.
    pub fn range(&self) -> Range<usize> {
        let start = self.cluster.offset as usize;
        start..start + self.cluster.len as usize
    }

    /// Returns an iterator over the glyphs for the cluster.
    pub fn glyphs(&self) -> Glyphs<'a> {
        let glyphs = self
            .cluster
            .glyphs(&self.layout.detailed_clusters, &self.layout.glyphs);
        Glyphs {
            layout: self.layout,
            iter: glyphs.iter(),
        }
    }

    /// Returns the advance of the cluster.
    pub fn advance(&self) -> f32 {
        self.cluster.advance(
            &self.layout.detailed_clusters,
            &self.layout.glyphs,
            &self.layout.detailed_glyphs,
        )
    }
}

/// Iterator over the clusters in a run.
#[derive(Clone)]
pub struct Clusters<'a> {
    layout: &'a LayoutData,
    iter: core::slice::Iter<'a, ClusterData>,
    rev: bool,
}

impl<'a> Iterator for Clusters<'a> {
    type Item = Cluster<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let data = if self.rev {
            self.iter.next_back()?
        } else {
            self.iter.next()?
        };
        Some(Cluster {
            layout: self.layout,
            cluster: *data,
        })
    }
}

impl<'a> DoubleEndedIterator for Clusters<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let data = self.iter.next_back()?;
        Some(Cluster {
            layout: self.layout,
            cluster: *data,
        })
    }
}

/// Collection of runs occupying a single line in a paragraph.
#[derive(Copy, Clone)]
pub struct Line<'a> {
    layout: &'a LayoutData,
    line_layout: &'a LineLayoutData,
    line: &'a LineData,
}

impl<'a> Line<'a> {
    pub fn new(layout: &'a Par, line_index: usize) -> Self {
        Self {
            layout: &layout.data,
            line_layout: &layout.line_data,
            line: &layout.line_data.lines[line_index],
        }
    }

    /// Returns the offset in line direction.
    pub fn offset(&self) -> f32 {
        self.line.x
    }

    /// Returns the baseline offset.
    pub fn baseline(&self) -> f32 {
        self.line.baseline
    }

    /// Returns the ascent of the line.
    pub fn ascent(&self) -> f32 {
        self.line.ascent
    }

    /// Returns the descent of the line.
    pub fn descent(&self) -> f32 {
        self.line.descent
    }

    /// Returns the leading of the line.
    pub fn leading(&self) -> f32 {
        self.line.leading
    }

    /// Returns the total advance of the line.
    pub fn advance(&self) -> f32 {
        self.line.width
    }

    /// Returns the total advance of the line excluding trailing whitespace.
    pub fn advance_without_trailing_whitespace(&self) -> f32 {
        let mut advance = self.line.width;
        for run in self.line_layout.runs[make_range(self.line.runs)]
            .iter()
            .rev()
        {
            if !run.trailing_whitespace {
                break;
            }
            for cluster in self.layout.clusters[make_range(run.clusters)].iter().rev() {
                if !cluster.info.is_whitespace() {
                    break;
                }
                advance -= Cluster {
                    layout: self.layout,
                    cluster: *cluster,
                }
                .advance();
            }
        }
        advance
    }

    /// Returns the size of the line (height for horizontal and width
    /// for vertical layouts).
    pub fn size(&self) -> f32 {
        self.line.ascent + self.line.descent + self.line.leading
    }

    /// Returns an iterator over the runs of the line.
    pub fn runs(&self) -> Runs<'a> {
        let range = self.line.runs.0 as usize..self.line.runs.1 as usize;
        Runs {
            layout: self.layout,
            iter: self.line_layout.runs[range].iter(),
        }
    }

    pub fn data(&self) -> &'a LineData {
        self.line
    }
}

/// Iterator over the lines of a paragraph.
#[derive(Clone)]
pub struct Lines<'a> {
    layout: &'a LayoutData,
    line_layout: &'a LineLayoutData,
    iter: core::slice::Iter<'a, LineData>,
}

impl<'a> Iterator for Lines<'a> {
    type Item = Line<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let line = self.iter.next()?;
        Some(Line {
            layout: self.layout,
            line_layout: self.line_layout,
            line,
        })
    }
}

pub fn make_range(r: (u32, u32)) -> Range<usize> {
    r.0 as usize..r.1 as usize
}

pub struct Session<'a> {
    pub(crate) engine: &'a mut Engine,
    pub(crate) dir_depth: u32,
    pub(crate) needs_bidi: bool,
    pub(crate) last_offset: usize,
    pub(crate) dir: Direction,
}

impl<'a> Session<'a> {
    #[inline]
    fn push_char(&mut self, ch: char) {
        let s = &mut self.engine.state;
        s.text.push(ch);
        s.text_frags.push(0);
        s.text_spans.push(0);
        s.text_offsets.push(0);
    }

    pub(super) fn push_span<'p, I>(&mut self, styles: I) -> Option<SpanId>
    where
        I: IntoIterator,
        I::Item: Borrow<SpanStyle<'p>>,
    {
        let s = &mut self.engine.state;
        let (id, dir) = s.push(&mut self.engine.font_ctx, styles)?;
        if let Some(dir) = dir {
            const LRI: char = '\u{2066}';
            const RLI: char = '\u{2067}';
            const FSI: char = '\u{2068}';
            match dir {
                Direction::Auto => self.push_char(FSI),
                Direction::Ltr => self.push_char(LRI),
                Direction::Rtl => self.push_char(RLI),
            }
            self.dir_depth += 1;
        }
        Some(id)
    }

    /// Pops the current span, restoring the styles of the parent.
    pub fn pop_span(&mut self) {
        let s = &mut self.engine.state;
        if let Some((_, dir_changed)) = s.pop() {
            if dir_changed {
                const PDI: char = '\u{2069}';
                self.dir_depth = self.dir_depth.saturating_sub(1);
                self.push_char(PDI);
            }
        }
    }

    /// Adds a text fragment to the paragraph.
    pub fn add_text(&mut self, text: &str) -> Option<()> {
        let s = &mut self.engine.state;
        let id = s.fragments.len();
        if id > MAX_ID.0 {
            return None;
        }
        let span_id = *s.span_stack.last()?;
        let span = s.spans.get(span_id.0)?;
        let mut offset = self.last_offset;
        macro_rules! push_char {
            ($ch: expr) => {{
                s.text.push($ch);
                s.text_offsets.push(offset);
                offset += ($ch).len_utf8();
            }};
        }
        let start = s.text.len();
        for ch in text.chars() {
            push_char!(ch);
        }
        let end = s.text.len();
        let break_shaping = if let Some(prev_frag) = s.fragments.last() {
            if prev_frag.is_text {
                if prev_frag.span == span_id {
                    false
                } else {
                    let s = s.spans.get(prev_frag.span.0)?;
                    s.font_size != span.font_size
                        || s.letter_spacing != span.letter_spacing
                        || s.lang != span.lang
                        || s.font != span.font
                        || s.font_features != span.font_features
                        || s.font_vars != span.font_vars
                }
            } else {
                true
            }
        } else {
            true
        };
        let len = end - start;
        s.text_frags.reserve(len);
        for _ in 0..len {
            s.text_frags.push(id);
        }
        s.text_spans.reserve(len);
        for _ in 0..len {
            s.text_spans.push(span_id.0);
        }

        s.fragments.push(FragmentData {
            span: span_id,
            is_text: true,
            break_shaping,
            start,
            end,
            features: span.font_features,
            vars: span.font_vars,
        });
        self.last_offset = offset;
        Some(())
    }

    /// Consumes the builder and fills the specified paragraph with the result.
    pub fn finish(mut self, par: Option<Par>) -> Par {
        let mut par = par.unwrap_or_default();
        par.clear();
        self.resolve(&mut par);
        self.engine.font_ctx.reset_group_state();
        par
    }
}

impl<'a> Session<'a> {
    fn resolve(&mut self, layout: &mut Par) {
        // Bit of a hack: add a single trailing space fragment to account for
        // empty paragraphs and to force an extra break if the paragraph ends
        // in a newline.
        let s = &mut self.engine.state;
        s.span_stack.push(SpanId(s.spans.len() - 1));
        self.add_text(" ");

        for _ in 0..self.dir_depth {
            const PDI: char = '\u{2069}';
            self.push_char(PDI);
        }

        let s = &mut self.engine.state;
        let mut analysis = analyze(s.text.iter());
        for (props, boundary) in analysis.by_ref() {
            s.text_info.push(CharInfo::new(props, boundary));
        }
        if analysis.needs_bidi_resolution() || self.dir != Direction::Ltr {
            let dir = match self.dir {
                Direction::Auto => None,
                Direction::Ltr => Some(BidiDirection::LeftToRight),
                Direction::Rtl => Some(BidiDirection::RightToLeft),
            };
            self.engine.bidi.resolve_with_types(
                &s.text,
                s.text_info.iter().map(|i| i.bidi_class()),
                dir,
            );
            self.needs_bidi = true;
        }
        self.itemize();
        self.shape(layout);
    }

    fn itemize(&mut self) {
        let s = &mut self.engine.state;
        let limit = s.text.len();
        if s.fragments.is_empty() || limit == 0 {
            return;
        }
        let mut last_script = s
            .text_info
            .iter()
            .map(|i| i.script())
            .find(|s| real_script(*s))
            .unwrap_or(Script::Latin);
        let levels = self.engine.bidi.levels();
        let mut last_frag = s.fragments.first().unwrap();
        let mut last_level = if self.needs_bidi {
            levels[last_frag.start]
        } else {
            0
        };
        let mut last_features = last_frag.features;
        let mut last_vars = last_frag.vars;
        let mut item = ItemData {
            script: last_script,
            level: last_level,
            start: last_frag.start,
            end: last_frag.start,
            features: last_features,
            vars: last_vars,
        };
        macro_rules! push_item {
            () => {
                if item.start < limit && item.start < item.end {
                    item.script = last_script;
                    item.level = last_level;
                    item.vars = last_vars;
                    item.features = last_features;
                    s.items.push(item);
                    item.start = item.end;
                }
            };
        }
        if self.needs_bidi {
            for frag in &s.fragments {
                if frag.break_shaping || frag.start != last_frag.end {
                    push_item!();
                    item.start = frag.start;
                    item.end = frag.start;
                }
                last_frag = frag;
                last_features = frag.features;
                last_vars = frag.vars;
                let range = frag.start..frag.end;
                for (&props, &level) in s.text_info[range.clone()].iter().zip(&levels[range]) {
                    let script = props.script();
                    let real = real_script(script);
                    if (script != last_script && real) || level != last_level {
                        //item.end += 1;
                        push_item!();
                        if real {
                            last_script = script;
                        }
                        last_level = level;
                    }
                    item.end += 1;
                }
            }
        } else {
            for frag in &s.fragments {
                if frag.break_shaping || frag.start != last_frag.end {
                    push_item!();
                    item.start = frag.start;
                    item.end = frag.start;
                }
                last_frag = frag;
                last_features = frag.features;
                last_vars = frag.vars;
                let range = frag.start..frag.end;
                for &props in &s.text_info[range] {
                    let script = props.script();
                    let real = real_script(script);
                    if script != last_script && real {
                        //item.end += 1;
                        push_item!();
                        if real {
                            last_script = script;
                        }
                    }
                    item.end += 1;
                }
            }
        }
        push_item!();
    }

    fn shape(&mut self, layout: &mut Par) {
        let s = &mut self.engine.state;
        let mut cluster = CharCluster::new();
        for item in &s.items {
            shape_item(
                &mut self.engine.font_ctx,
                &mut self.engine.scx,
                &s,
                item,
                &mut cluster,
                layout,
            );
        }
        layout.apply_spacing(&s.spans);
        layout.finish();
    }
}

/// Data that describes a span.
#[derive(Clone)]
pub struct SpanData {
    /// Identifier of the span.
    pub(super) id: SpanId,
    /// Identifier of the parent span.
    pub(super) parent: Option<SpanId>,
    /// Identifier of first child of the span.
    pub(super) first_child: Option<SpanId>,
    /// Identifier of last child of the span.
    pub(super) last_child: Option<SpanId>,
    /// Identifier of next sibling of the span.
    pub(super) next: Option<SpanId>,
    /// Text direction.
    pub dir: Direction,
    /// Is the direction different from the parent?
    pub dir_changed: bool,
    /// Text language.
    pub lang: Option<Language>,
    /// Internal identifier for a list of font families and attributes.
    pub font: FontGroupId,
    /// Font family.
    pub font_family: FamilyList,
    /// Font attributes.
    pub font_attrs: (Stretch, Weight, Style),
    /// Font size in ppem.
    pub font_size: f32,
    /// Font features.
    pub font_features: FontSettingKey,
    /// Font variations.
    pub font_vars: FontSettingKey,
    /// Additional spacing between letters (clusters) of text.
    pub letter_spacing: f32,
    /// Additional spacing between words of text.
    pub word_spacing: f32,
    /// Multiplicative line spacing factor.
    pub line_spacing: f32,
    /// Enable underline decoration.
    pub underline: bool,
    /// Offset of an underline.
    pub underline_offset: Option<f32>,
    /// Thickness of an underline.
    pub underline_size: Option<f32>,
}

/// Build State
#[derive(Default)]
pub struct BuilderState {
    /// Combined text.
    pub text: Vec<char>,
    /// Fragment index per character.
    pub text_frags: Vec<usize>,
    /// Span index per character.
    pub text_spans: Vec<usize>,
    /// Character info per character.
    pub text_info: Vec<CharInfo>,
    /// Offset of each character relative to its fragment.
    pub text_offsets: Vec<usize>,
    /// Collection of all spans, in order of span identifier.
    pub spans: Vec<SpanData>,
    /// Stack of spans.
    pub span_stack: Vec<SpanId>,
    /// Font feature setting cache.
    pub features: FontSettingCache<u16>,
    /// Font variation setting cache.
    pub vars: FontSettingCache<f32>,
    /// Collection of fragments.
    pub fragments: Vec<FragmentData>,
    /// Collection of items.
    pub items: Vec<ItemData>,
    // /// User specified scale.
    // pub scale: f32,
}

impl BuilderState {
    /// Creates a new layout state.
    pub fn new() -> Self {
        Self::default()
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.text_frags.clear();
        self.text_spans.clear();
        self.text_info.clear();
        self.text_offsets.clear();
        self.spans.clear();
        self.span_stack.clear();
        self.features.clear();
        self.vars.clear();
        self.fragments.clear();
        self.items.clear();
    }

    pub fn begin(
        &mut self,
        dir: Direction,
        lang: Option<Language>,
        // scale: f32,
        _base_offset: usize,
    ) {
        self.spans.push(SpanData {
            id: SpanId(0),
            parent: None,
            first_child: None,
            last_child: None,
            next: None,
            dir,
            dir_changed: false,
            lang,
            font: FontGroupId(!0),
            font_family: FamilyList::new(""),
            font_attrs: (Stretch::NORMAL, Weight::NORMAL, Style::Normal),
            font_size: 16.,
            font_features: EMPTY_FONT_SETTINGS,
            font_vars: EMPTY_FONT_SETTINGS,
            letter_spacing: 0.,
            word_spacing: 0.,
            line_spacing: 1.,
            underline: false,
            underline_offset: None,
            underline_size: None,
        });
        self.span_stack.push(SpanId(0));
    }

    /// Pushes a new span with the specified properties. Returns the new
    /// span identifier and a value indicating a new direction, if any.
    pub(super) fn push<'a, I>(
        &mut self,
        fcx: &mut FontContext,
        // scale: f32,
        styles: I,
    ) -> Option<(SpanId, Option<Direction>)>
    where
        I: IntoIterator,
        I::Item: Borrow<SpanStyle<'a>>,
    {
        let next_id = SpanId(self.spans.len());
        if next_id > MAX_ID {
            return None;
        }
        let parent_id = *self.span_stack.last()?;
        let parent = self.spans.get_mut(parent_id.0)?;
        let mut span = parent.clone();
        let last_child = if let Some(last_child) = parent.last_child {
            parent.last_child = Some(next_id);
            Some(last_child)
        } else {
            parent.first_child = Some(next_id);
            parent.last_child = Some(next_id);
            None
        };
        if let Some(last_child) = last_child {
            let prev_sibling = self.spans.get_mut(last_child.0)?;
            prev_sibling.next = Some(next_id);
        }
        span.id = next_id;
        span.parent = Some(parent_id);
        span.dir_changed = false;
        let parent_dir = span.dir;
        let mut font_changed = false;
        for s in styles {
            use SpanStyle as S;
            match s.borrow() {
                S::Direction(dir) => {
                    if *dir != parent_dir {
                        span.dir = *dir;
                        span.dir_changed = true;
                    } else {
                        span.dir = *dir;
                        span.dir_changed = false;
                    }
                }
                S::Language(lang) => {
                    span.lang = Some(*lang);
                }
                S::FamilyList(families) => {
                    if families.key() != span.font_family.key() {
                        span.font_family = families.clone();
                        font_changed = true;
                    }
                }
                S::Stretch(value) => {
                    if *value != span.font_attrs.0 {
                        span.font_attrs.0 = *value;
                        font_changed = true;
                    }
                }
                S::Weight(value) => {
                    if *value != span.font_attrs.1 {
                        span.font_attrs.1 = *value;
                        font_changed = true;
                    }
                }
                S::Style(value) => {
                    if *value != span.font_attrs.2 {
                        span.font_attrs.2 = *value;
                        font_changed = true;
                    }
                }
                S::Size(size) => {
                    span.font_size = *size;
                }
                S::Features(features) => {
                    span.font_features = self.features.add(features.iter().copied());
                }
                S::Variations(vars) => {
                    span.font_vars = self.vars.add(vars.iter().copied());
                }
                S::LetterSpacing(spacing) => {
                    span.letter_spacing = *spacing;
                }
                S::WordSpacing(spacing) => {
                    span.word_spacing = *spacing;
                }
                S::LineSpacing(spacing) => {
                    span.line_spacing = *spacing;
                }
                S::Underline(enable) => {
                    span.underline = *enable;
                }
                S::UnderlineOffset(offset) => {
                    span.underline_offset = *offset;
                }
                S::UnderlineSize(size) => span.underline_size = *size,
            }
        }
        if font_changed {
            span.font = fcx.register_group(
                span.font_family.names(),
                span.font_family.key(),
                span.font_attrs.into(),
            );
        }
        let dir = if span.dir_changed {
            Some(span.dir)
        } else {
            None
        };
        self.spans.push(span);
        self.span_stack.push(next_id);
        Some((next_id, dir))
    }

    /// Pops the most recent span from the stack. Returns true if
    /// the direction changed.
    pub fn pop(&mut self) -> Option<(SpanId, bool)> {
        if self.span_stack.len() > 1 {
            let id = self.span_stack.pop().unwrap();
            Some((id, self.spans[id.0].dir_changed))
        } else {
            None
        }
    }
}

/// Index into a font setting cache.
pub type FontSettingKey = u32;

/// Sentinel for an empty set of font settings.
pub const EMPTY_FONT_SETTINGS: FontSettingKey = !0;

/// Cache of tag/value pairs for font settings.
#[derive(Default)]
pub struct FontSettingCache<T: Copy + PartialOrd + PartialEq> {
    settings: Vec<Setting<T>>,
    lists: Vec<FontSettingList>,
    tmp: Vec<Setting<T>>,
}

impl<T: Copy + PartialOrd + PartialEq> FontSettingCache<T> {
    pub fn add<I>(&mut self, settings: I) -> FontSettingKey
    where
        I: Iterator,
        I::Item: Into<Setting<T>>,
    {
        self.tmp.clear();
        self.tmp.extend(settings.map(|v| v.into()));
        let len = self.tmp.len();
        if len == 0 {
            return EMPTY_FONT_SETTINGS;
        }
        self.tmp.sort_unstable_by(|a, b| a.tag.cmp(&b.tag));
        'outer: for (i, list) in self.lists.iter().enumerate() {
            let other = list.get(&self.settings);
            if other.len() != len {
                continue;
            }
            for (a, b) in self.tmp.iter().zip(other) {
                if a.tag != b.tag || a.value != b.value {
                    continue 'outer;
                }
            }
            return i as u32;
        }
        let key = self.lists.len() as u32;
        let start = self.settings.len() as u32;
        self.settings.extend_from_slice(&self.tmp);
        let end = self.settings.len() as u32;
        self.lists.push(FontSettingList { start, end });
        key
    }

    pub fn get(&self, key: u32) -> &[Setting<T>] {
        if key == !0 {
            &[]
        } else {
            self.lists
                .get(key as usize)
                .map(|list| list.get(&self.settings))
                .unwrap_or(&[])
        }
    }

    pub fn clear(&mut self) {
        self.settings.clear();
        self.lists.clear();
        self.tmp.clear();
    }
}

#[inline]
fn real_script(script: Script) -> bool {
    script != Script::Common && script != Script::Inherited && script != Script::Unknown
}

/// Range within a font setting cache.
#[derive(Copy, Clone)]
struct FontSettingList {
    pub start: u32,
    pub end: u32,
}

impl FontSettingList {
    pub fn get<T>(self, elements: &Vec<T>) -> &[T] {
        elements
            .get(self.start as usize..self.end as usize)
            .unwrap_or(&[])
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FragmentData {
    pub span: SpanId,
    pub break_shaping: bool,
    pub is_text: bool,
    pub start: usize,
    pub end: usize,
    pub features: FontSettingKey,
    pub vars: FontSettingKey,
}

#[derive(Copy, Clone, Debug)]
pub struct ItemData {
    /// Script of the item.
    pub script: Script,
    /// Bidi level of the item.
    pub level: u8,
    /// Offset of the text.
    pub start: usize,
    /// End of the text.
    pub end: usize,
    /// Font features.
    pub features: FontSettingKey,
    /// Font variations.
    pub vars: FontSettingKey,
}

/// Shape
struct ShapeState<'a> {
    state: &'a BuilderState,
    features: &'a [Setting<u16>],
    synth: Synthesis,
    vars: &'a [Setting<f32>],
    script: Script,
    level: u8,
    span_index: usize,
    span: &'a SpanData,
    font_id: FontGroupId,
    font: Option<Font>,
    size: f32,
}

fn shape_item(
    fcx: &mut FontContext,
    scx: &mut ShapeContext,
    state: &BuilderState,
    item: &ItemData,
    cluster: &mut CharCluster,
    layout: &mut Par,
) -> Option<()> {
    let dir = if item.level & 1 != 0 {
        swash::shape::Direction::RightToLeft
    } else {
        swash::shape::Direction::LeftToRight
    };

    let range = item.start..item.end;
    let span_index = state.text_spans[item.start];
    let span = state.spans.get(span_index)?;
    let features = state.features.get(item.features);
    let vars = state.vars.get(item.vars);
    let mut shape_state = ShapeState {
        script: item.script,
        level: item.level,
        features,
        vars,
        synth: Synthesis::default(),
        state,
        span_index,
        span,
        font_id: span.font,
        font: None,
        size: span.font_size,
    };
    fcx.select_group(shape_state.font_id);
    fcx.select_fallbacks(item.script, shape_state.span.lang.as_ref());

    if item.level & 1 != 0 {
        let chars = state.text[range.clone()]
            .iter()
            .zip(&state.text_offsets[range.clone()])
            .zip(&state.text_spans[range.clone()])
            .zip(&state.text_info[range])
            .map(|z| {
                use swash::text::Codepoint;
                let (((&ch, &offset), &span_index), &info) = z;
                let ch = ch.mirror().unwrap_or(ch);
                Token {
                    ch,
                    offset: offset as u32,
                    len: ch.len_utf8() as u8,
                    info,
                    data: span_index as u32,
                }
            });

        let mut parser = Parser::new(item.script, chars);
        if !parser.next(cluster) {
            return Some(());
        }
        shape_state.font = fcx.map_cluster(cluster, &mut shape_state.synth);
        while shape_clusters(
            fcx,
            scx,
            &mut shape_state,
            &mut parser,
            cluster,
            dir,
            layout,
        ) {}
    } else {
        let chars = state.text[range.clone()]
            .iter()
            .zip(&state.text_offsets[range.clone()])
            .zip(&state.text_spans[range.clone()])
            .zip(&state.text_info[range])
            .map(|z| {
                let (((&ch, &offset), &span_index), &info) = z;
                Token {
                    ch,
                    offset: offset as u32,
                    len: ch.len_utf8() as u8,
                    info,
                    data: span_index as u32,
                }
            });

        let mut parser = Parser::new(item.script, chars);
        if !parser.next(cluster) {
            return Some(());
        }
        shape_state.font = fcx.map_cluster(cluster, &mut shape_state.synth);
        while shape_clusters(
            fcx,
            scx,
            &mut shape_state,
            &mut parser,
            cluster,
            dir,
            layout,
        ) {}
    }
    Some(())
}

fn shape_clusters<I>(
    fcx: &mut FontContext,
    scx: &mut ShapeContext,
    state: &mut ShapeState,
    parser: &mut Parser<I>,
    cluster: &mut CharCluster,
    dir: swash::shape::Direction,
    layout: &mut Par,
) -> bool
where
    I: Iterator<Item = Token> + Clone,
{
    if state.font.is_none() {
        return false;
    }
    let mut shaper = scx
        .builder(state.font.as_ref().unwrap().as_ref())
        .script(state.script)
        .language(state.span.lang)
        .direction(dir)
        .size(state.size)
        .features(state.features.iter().copied())
        .variations(state.synth.variations().iter().copied())
        .variations(state.vars.iter().copied())
        .build();
    let mut synth = Synthesis::default();
    loop {
        shaper.add_cluster(cluster);
        if !parser.next(cluster) {
            layout.push_run(
                &state.state.spans,
                state.font.clone().unwrap(),
                state.size,
                state.level,
                shaper,
            );
            return false;
        }
        let cluster_span = cluster.user_data();
        if cluster_span as usize != state.span_index {
            state.span_index = cluster_span as usize;
            state.span = state.state.spans.get(cluster_span as usize).unwrap();
            if state.span.font != state.font_id {
                state.font_id = state.span.font;
                fcx.select_group(state.font_id);
            }
        }
        let next_font = fcx.map_cluster(cluster, &mut synth);

        if next_font != state.font || synth != state.synth {
            layout.push_run(
                &state.state.spans,
                state.font.clone().unwrap(),
                state.size,
                state.level,
                shaper,
            );
            state.font = next_font;
            state.synth = synth;
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        doc::{Direction, Doc, SpanStyle},
        engine::Engine,
        line_breaker::Alignment,
    };

    fn layout_doc(doc: &Doc<'_>) -> Par {
        let mut engine = Engine::new();
        let mut session = engine.start(Direction::Ltr, None, 0);
        session.process(doc);
        session.finish(None)
    }

    fn cluster_advance(par: &Par) -> f32 {
        par.data
            .clusters
            .iter()
            .map(|cluster| {
                cluster.advance(
                    &par.data.detailed_clusters,
                    &par.data.glyphs,
                    &par.data.detailed_glyphs,
                )
            })
            .sum()
    }

    #[test]
    fn default_font_shapes_plain_text() {
        let doc = Doc::simple(Vec::<SpanStyle<'_>>::new(), "abc");
        let par = layout_doc(&doc);

        assert!(!par.data.runs.is_empty());
        assert!(!par.data.clusters.is_empty());
        assert!(cluster_advance(&par) > 0.);
    }

    #[test]
    fn span_split_run_advances_do_not_accumulate() {
        let mut builder = Doc::builder();
        builder.enter_span(Vec::<SpanStyle<'_>>::new());
        builder.enter_span([SpanStyle::Underline(false)]);
        builder.add_text("ab");
        builder.leave_span();
        builder.enter_span([SpanStyle::Underline(true)]);
        builder.add_text("cd");
        builder.leave_span();
        builder.leave_span();
        let doc = builder.build();
        let par = layout_doc(&doc);

        assert!(par.data.runs.len() >= 2);
        let run_advance: f32 = par.data.runs.iter().map(|run| run.advance).sum();
        let shaped_advance = cluster_advance(&par);

        assert!((run_advance - shaped_advance).abs() < 0.01);
    }

    #[test]
    fn overwide_clusters_are_not_skipped_when_breaking_lines() {
        let doc = Doc::simple(Vec::<SpanStyle<'_>>::new(), "abc");
        let mut par = layout_doc(&doc);
        let logical_clusters = par.data.clusters.len();

        par.break_lines().break_remaining(0., Alignment::Start);

        assert_eq!(par.line_data.clusters.len(), logical_clusters);
    }
}
