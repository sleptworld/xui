//! A focusable viewport onto a virtual document of text blocks, laid out and
//! painted by the runtime and edited by a model an outside crate implements.
//!
//! The surface's size comes from its style, as a canvas's does: taffy never
//! measures its text. After layout, the runtime syncs it (see
//! [`TextSurfaceState::sync`]): it applies the model's changes to a virtual
//! document in the text host, one paragraph per block, lays out the blocks in
//! and around the viewport, records real heights in place of estimates and
//! anchors the scroll so a block whose height settles does not move what is on
//! screen. Painting draws the laid-out blocks and the model's decorations.
//! Events reach the model as [`SurfaceInput`], with pointer positions already
//! resolved to text positions, and a [`SurfaceCx`] for geometry and the
//! platform. See `docs/text-surface-api.md`.

pub mod model;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use rustc_hash::{FxHashMap, FxHashSet};
use xui_interface::events::{KeyState, PointerButton, RawEvent, SemanticEvent, XuiPointerId};
use xui_interface::{
    Affine, Affinity, Bounds, Color, ComputedColorStyle, ComputedStyle, ComputedTextStyle,
    EventRef, EventResult, LineHeight, NodeId, Point, Rect, Size, TextBackend, TextBoxStyle,
    TextCaret, TextContent, TextInputPurpose, TextInputSession, TextLayoutConstraints,
    TextLayoutInput, TextOffset, TextPaintProps, TextPaintStyle, TextRange, TextVerticalAlign,
    WidgetUpdateFlags,
};
use xui_macros::widget;

use super::CanvasInvalidator;
use super::roles::{DefaultFocus, HandleEvent, PaintContent, PlatformTextInput};
use crate::event_system::EventContext;
use crate::node::{Draft, WidgetT};
use crate::render::{ClipShape, Primitive, RenderTreeWriter, Shape, ShapePrimitive, TextPrimitive};
use crate::text::{
    ParagraphId, TextDocumentId, TextHost, TextLayoutQuery, TextUnitLookup, TextUnitRef,
};
use crate::widgets::text_input_content_box;

pub use model::{
    BlockChange, BlockId, DecorKind, DecorLayer, SurfaceBlock, SurfaceChanges, SurfaceCx,
    SurfaceDecor, SurfaceInput, SurfacePointer, SurfacePointerKind, SurfacePosition, SurfaceRange,
    TextSurfaceHandle, TextSurfaceModel, UnderlineStyle, WeakTextSurfaceHandle,
};
use model::{SurfaceGeometry, SurfaceRequests};

/// How long the caret stays in each half of a blink, as in a text input.
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);
/// Clicks closer than this, in time and in distance, count as one more click.
const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE: f32 = 4.0;
/// Enough passes for estimates to settle and a reveal to land; each pass
/// shapes only blocks the one before did not.
const MAX_SYNC_PASSES: usize = 6;

/// Whether blocks wrap at the viewport's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SurfaceWrap {
    #[default]
    Width,
    /// Each block is one line per hard break, as wide as it needs; the
    /// surface scrolls sideways.
    None,
}

/// A viewport onto a [`TextSurfaceModel`]'s blocks. It paints its own text
/// and decorations, so it selects no decoration; put a box around it.
#[widget(
    style(
        box_size, margin, padding, position, display, typography, visibility, transform, effects,
        cursor
    ),
    behavior(
        focus,
        accessibility,
        shortcuts,
        motion,
        focus_binding,
        size_observation,
        events
    )
)]
#[props(derive(Debug, Clone, PartialEq))]
pub struct TextSurface {
    /// What it shows and how it is edited. Swapping in another handle reads
    /// the new model from the start.
    #[default(TextSurfaceHandle::empty())]
    #[dirty(PAINT_OUTPUT | TEXT_SHAPE)]
    pub model: TextSurfaceHandle,
    /// Takes it out of focus navigation, and stops input reaching the model.
    #[default]
    #[dirty(PAINT_OUTPUT)]
    pub disabled: bool,
    #[default]
    #[dirty(PAINT_OUTPUT | TEXT_SHAPE)]
    pub wrap: SurfaceWrap,
    /// How many viewports above and below are kept laid out, for geometry and
    /// scrolling. At least one.
    #[default(1.0)]
    #[dirty(TEXT_SHAPE)]
    pub overscan: f32,
}

impl WidgetT for TextSurface {
    type State = TextSurfaceState;

    fn create_state(props: &TextSurfaceProps) -> TextSurfaceState {
        TextSurfaceState::new(props)
    }

    fn update(
        _: &TextSurfaceProps,
        new: &TextSurfaceProps,
        state: &mut TextSurfaceState,
    ) -> WidgetUpdateFlags {
        state.follow(new);
        WidgetUpdateFlags::empty()
    }
}

/// A text surface, at least 40 × 20 and showing the text cursor.
pub fn text_surface() -> Draft<TextSurface> {
    Draft::new(TextSurfaceProps::builder().build())
        .with::<text_surface::min_width>(40.0)
        .with::<text_surface::min_height>(20.0)
        .with::<text_surface::cursor>(xui_interface::CursorIcon::Text)
}

impl DefaultFocus for TextSurfaceState {
    fn focusable(&self) -> bool {
        !self.disabled
    }
}

/// What every block was laid out under. When it moves, every measured height
/// is stale.
#[derive(Debug, Clone, PartialEq)]
struct LayoutBasis {
    constraints: TextLayoutConstraints,
    style: ComputedTextStyle,
}

/// A laid-out block in the viewport, as it is painted.
#[derive(Debug, Clone)]
struct PaintedBlock {
    document: TextDocumentId,
    paragraph: ParagraphId,
    /// Its top, in content coordinates.
    top: f32,
    size: Size<f32>,
    /// Its text, to turn byte offsets into the char offsets carets take.
    text: TextContent,
    paint: TextPaintProps,
    /// The carets in it. They are painted with its glyphs, in its text
    /// primitive, never as shapes of their own: a shape over text changes
    /// how a renderer composes everything after it, so a blinking one made
    /// the whole page's text change with each blink.
    carets: Vec<TextCaret>,
    revision: u64,
}

/// A decoration resolved to one rectangle, in content coordinates.
#[derive(Debug, Clone, Copy)]
struct DecorRect {
    rect: Rect,
    color: Color,
    layer: DecorLayer,
}

/// A text surface's state while mounted: its virtual document, scroll,
/// what it paints, and its input state.
pub struct TextSurfaceState {
    handle: TextSurfaceHandle,
    binding: Option<(NodeId, CanvasInvalidator)>,
    disabled: bool,
    wrap: SurfaceWrap,
    overscan: f32,
    /// Set when the next sync must read the whole model again.
    needs_reset: bool,

    document: Option<TextDocumentId>,
    paragraphs: FxHashMap<BlockId, ParagraphId>,
    blocks: FxHashMap<ParagraphId, BlockId>,
    basis: Option<LayoutBasis>,
    /// The blocks whose layouts are active.
    resident: Vec<ParagraphId>,
    /// How far the content is scrolled, in content coordinates.
    scroll: Point,
    /// The block at the top of the viewport and how far into it the top is,
    /// so the scroll follows it when heights above it settle.
    anchor: Option<(ParagraphId, f32)>,
    reveal: Option<(SurfacePosition, f32)>,
    viewport: Size<f32>,
    /// The widest block laid out so far, for scrolling sideways.
    content_width: f32,
    document_height: f32,
    painted: Vec<PaintedBlock>,
    decor: Vec<DecorRect>,
    /// The primary caret, in content coordinates, for the IME.
    ime_caret: Option<Rect>,

    focused: bool,
    caret_on: bool,
    drag: Option<XuiPointerId>,
    last_click: Option<(Instant, Point, u8)>,
}

impl std::fmt::Debug for TextSurfaceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextSurfaceState")
            .field("blocks", &self.paragraphs.len())
            .field("resident", &self.resident.len())
            .field("scroll", &self.scroll)
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

/// What a sync needs from the runtime.
pub(crate) struct SurfaceFrame<'a, T: TextBackend> {
    pub owner: NodeId,
    /// The node's size inside its padding.
    pub viewport: Size<f32>,
    pub style: &'a ComputedStyle,
    pub measurer: &'a mut TextHost<T>,
}

impl TextSurfaceState {
    fn new(props: &TextSurfaceProps) -> Self {
        Self {
            handle: props.model.clone(),
            binding: None,
            disabled: props.disabled,
            wrap: props.wrap,
            overscan: props.overscan,
            needs_reset: true,
            document: None,
            paragraphs: FxHashMap::default(),
            blocks: FxHashMap::default(),
            basis: None,
            resident: Vec::new(),
            scroll: Point::new(0.0, 0.0),
            anchor: None,
            reveal: None,
            viewport: Size::new(0.0, 0.0),
            content_width: 0.0,
            document_height: 0.0,
            painted: Vec::new(),
            decor: Vec::new(),
            ime_caret: None,
            focused: false,
            caret_on: true,
            drag: None,
            last_click: None,
        }
    }

    pub(crate) fn bind(&mut self, id: NodeId, invalidator: CanvasInvalidator) {
        self.handle.bind(id, invalidator.clone());
        self.binding = Some((id, invalidator));
    }

    pub(crate) fn unbind(&mut self) {
        if let Some((id, _)) = self.binding.take() {
            self.handle.unbind(id);
        }
    }

    /// Takes on `next`'s props, moving the binding to a new model.
    fn follow(&mut self, next: &TextSurfaceProps) {
        self.disabled = next.disabled;
        self.wrap = next.wrap;
        self.overscan = next.overscan;
        if self.handle.same_handle(&next.model) {
            return;
        }
        if let Some((id, invalidator)) = self.binding.clone() {
            self.handle.unbind(id);
            next.model.bind(id, invalidator);
        }
        self.handle = next.model.clone();
        self.needs_reset = true;
        self.scroll = Point::new(0.0, 0.0);
        self.anchor = None;
        self.reveal = None;
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> Point {
        self.scroll
    }

    #[cfg(test)]
    pub(crate) fn painted_blocks(&self) -> Vec<(ParagraphId, f32)> {
        self.painted
            .iter()
            .map(|block| (block.paragraph, block.top))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn resident_count(&self) -> usize {
        self.resident.len()
    }

    #[cfg(test)]
    pub(crate) fn paragraph_of(&self, block: BlockId) -> Option<ParagraphId> {
        self.paragraphs.get(&block).copied()
    }

    /// The carets painted with each block's text, while focused.
    #[cfg(test)]
    pub(crate) fn painted_carets(&self) -> Vec<(ParagraphId, TextCaret)> {
        self.painted
            .iter()
            .flat_map(|block| block.carets.iter().map(|caret| (block.paragraph, *caret)))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn decoration_rects(&self) -> Vec<Rect> {
        self.decor.iter().map(|decor| decor.rect).collect()
    }

    /// Brings the virtual document up to date with the model, lays out what
    /// is in and around the viewport, and records what to paint.
    pub(crate) fn sync<T: TextBackend>(&mut self, frame: SurfaceFrame<'_, T>) {
        let SurfaceFrame {
            owner,
            viewport,
            style,
            measurer,
        } = frame;
        self.viewport = viewport;
        let model_rc = self.handle.model();
        let mut model = model_rc.borrow_mut();
        let changes = model.take_changes();

        let text_style = style.text.clone();
        let line_height = line_height_px(&text_style);
        let constraints = match self.wrap {
            SurfaceWrap::Width => TextLayoutConstraints::max_width(viewport.width.max(0.0)),
            SurfaceWrap::None => TextLayoutConstraints::UNBOUNDED,
        };
        let basis = LayoutBasis {
            constraints,
            style: text_style.clone(),
        };
        let font_context = measurer.font_epoch();

        let document_lost = self
            .document
            .is_none_or(|document| measurer.document_len(owner, document).is_none());
        if self.needs_reset || document_lost || changes == SurfaceChanges::Reset {
            self.reset(owner, &*model, measurer, line_height);
            self.basis = Some(basis.clone());
        } else if let SurfaceChanges::Edits(edits) = changes {
            self.apply_edits(owner, &*model, measurer, &edits, line_height);
        }
        let document = self.document.expect("a sync always leaves a document");

        if self.basis.as_ref() != Some(&basis) {
            // Every height was measured at another width or in another style.
            for (&paragraph, &block) in &self.blocks {
                let estimate = model.estimated_lines(block).max(1) as f32 * line_height;
                measurer.update_paragraph_estimate(owner, document, paragraph, estimate);
                measurer.update_paragraph_height(owner, document, paragraph, None);
            }
            self.content_width = 0.0;
            self.basis = Some(basis);
        }

        if let Some((paragraph, offset)) = self.anchor
            && let Some(top) = measurer.paragraph_top(owner, document, paragraph)
        {
            self.scroll.y = top + offset;
        }

        let overscan = self.overscan.max(1.0) * viewport.height.max(line_height);
        let mut shaped: FxHashMap<ParagraphId, (crate::text::TextLayoutHandle, SurfaceBlock)> =
            FxHashMap::default();
        let mut range = 0..0;
        for _ in 0..MAX_SYNC_PASSES {
            let height = measurer.document_height(owner, document).unwrap_or(0.0);
            self.scroll.y = self
                .scroll
                .y
                .clamp(0.0, (height - viewport.height).max(0.0));
            range = visible_range(
                measurer,
                owner,
                document,
                self.scroll.y - overscan,
                self.scroll.y + viewport.height + overscan,
            );
            let mut settled = true;
            for paragraph in measurer.paragraphs_in(owner, document, range.clone()) {
                if shaped.contains_key(&paragraph) {
                    continue;
                }
                let Some(&block_id) = self.blocks.get(&paragraph) else {
                    continue;
                };
                let block = model.block(block_id);
                let (shape_runs, _) = block.runs.split(&text_style);
                let input = TextLayoutInput::new(
                    block.text.clone(),
                    constraints,
                    text_style.clone(),
                    block.paragraph.clone(),
                    TextBoxStyle::default(),
                    font_context,
                )
                .with_shape_runs(shape_runs);
                let Some(handle) = measurer.activate_paragraph(owner, document, paragraph, input)
                else {
                    continue;
                };
                let size = measurer
                    .layout(handle)
                    .map_or(Size::new(0.0, line_height), |layout| layout.size());
                let measured = size.height.max(line_height);
                let previous = measurer
                    .paragraph(owner, document, paragraph)
                    .map(|info| info.height());
                if previous.is_none_or(|previous| (previous - measured).abs() > 0.01) {
                    measurer.update_paragraph_height(owner, document, paragraph, Some(measured));
                    settled = false;
                }
                self.content_width = self.content_width.max(size.width);
                shaped.insert(paragraph, (handle, block));
            }

            if let Some((at, margin)) = self.reveal {
                match self.reveal_scroll(measurer, owner, document, at, margin, &shaped) {
                    Reveal::Done(scroll) => {
                        self.reveal = None;
                        if scroll != self.scroll {
                            self.scroll = scroll;
                            settled = false;
                        }
                    }
                    Reveal::Jump(y) => {
                        self.scroll.y = y;
                        settled = false;
                    }
                    Reveal::Gone => self.reveal = None,
                }
            }
            if settled {
                break;
            }
            // The scroll follows the block at the top as heights settle.
            if let Some((paragraph, offset)) =
                anchor_at(measurer, owner, document, self.scroll.y, self.anchor_hint())
                && let Some(top) = measurer.paragraph_top(owner, document, paragraph)
            {
                self.scroll.y = top + offset;
            }
        }
        self.reveal = None;

        let max_x = match self.wrap {
            SurfaceWrap::Width => 0.0,
            SurfaceWrap::None => (self.content_width - viewport.width).max(0.0),
        };
        self.scroll.x = self.scroll.x.clamp(0.0, max_x);
        self.document_height = measurer.document_height(owner, document).unwrap_or(0.0);
        self.anchor = anchor_at(measurer, owner, document, self.scroll.y, None);

        // Layouts that fell out of range may be evicted from now on.
        let resident = measurer.paragraphs_in(owner, document, range);
        let kept: FxHashSet<ParagraphId> = resident.iter().copied().collect();
        for paragraph in self.resident.drain(..) {
            if !kept.contains(&paragraph)
                && let Some(unit) = measurer.paragraph_unit(owner, document, paragraph)
            {
                measurer.clear_active_unit(unit);
            }
        }
        self.resident = resident;

        self.painted.clear();
        let view_top = self.scroll.y;
        let view_bottom = self.scroll.y + viewport.height;
        let paint_style = TextPaintStyle::from_computed(&text_style);
        let width = match self.wrap {
            SurfaceWrap::Width => viewport.width,
            SurfaceWrap::None => self.content_width.max(viewport.width),
        };
        for &paragraph in &self.resident {
            let Some((handle, block)) = shaped.get(&paragraph) else {
                continue;
            };
            let Some(top) = measurer.paragraph_top(owner, document, paragraph) else {
                continue;
            };
            let height = measurer
                .paragraph(owner, document, paragraph)
                .map_or(line_height, |info| info.height());
            if top + height < view_top || top > view_bottom {
                continue;
            }
            let (_, paint_runs) = block.runs.split(&text_style);
            self.painted.push(PaintedBlock {
                document,
                paragraph,
                top,
                size: Size::new(width, height),
                text: block.text.clone(),
                paint: TextPaintProps::new(paint_style).with_runs(paint_runs),
                carets: Vec::new(),
                revision: handle_revision(*handle),
            });
        }

        let visible: Vec<BlockId> = self
            .painted
            .iter()
            .filter_map(|block| self.blocks.get(&block.paragraph).copied())
            .collect();
        let decorations = model.decorations(&visible);
        let (decor, carets) =
            self.resolve_decorations(measurer, owner, document, &decorations, &text_style);
        self.decor = decor;
        for (position, caret) in carets {
            self.painted[position].carets.push(caret);
        }
        self.ime_caret = model
            .primary_caret()
            .and_then(|at| self.geometry(owner, &*measurer).caret_rect(at));
    }

    /// The anchor before this sync moved anything, when it is still in the
    /// document.
    fn anchor_hint(&self) -> Option<ParagraphId> {
        self.anchor.map(|(paragraph, _)| paragraph)
    }

    fn geometry<'a>(
        &'a self,
        owner: NodeId,
        lookup: &'a dyn TextUnitLookup,
    ) -> SurfaceGeometry<'a> {
        SurfaceGeometry {
            owner,
            document: self.document,
            lookup: Some(lookup),
            paragraphs: &self.paragraphs,
            blocks: &self.blocks,
            scroll: self.scroll,
            viewport: self.viewport,
        }
    }

    /// Drops the document and reads the model again from the start.
    fn reset<T: TextBackend>(
        &mut self,
        owner: NodeId,
        model: &dyn TextSurfaceModel,
        measurer: &mut TextHost<T>,
        line_height: f32,
    ) {
        if let Some(document) = self.document.take() {
            measurer.remove_document(owner, document);
        }
        self.paragraphs.clear();
        self.blocks.clear();
        self.resident.clear();
        self.anchor = None;
        self.content_width = 0.0;
        self.needs_reset = false;
        let document = measurer.create_document(owner);
        self.document = Some(document);
        for index in 0..model.block_count() {
            let id = model.block_at(index);
            self.insert_block(owner, model, measurer, index, id, line_height);
        }
    }

    fn insert_block<T: TextBackend>(
        &mut self,
        owner: NodeId,
        model: &dyn TextSurfaceModel,
        measurer: &mut TextHost<T>,
        index: usize,
        id: BlockId,
        line_height: f32,
    ) {
        let Some(document) = self.document else {
            return;
        };
        if let Some(previous) = self.paragraphs.remove(&id) {
            self.blocks.remove(&previous);
            measurer.remove_paragraph(owner, document, previous);
        }
        let estimate = model.estimated_lines(id).max(1) as f32 * line_height;
        let empty = TextRange::new(TextOffset::byte_offset(0), TextOffset::byte_offset(0));
        if let Some(info) = measurer.insert_paragraph(owner, document, index, empty, 0, 0, estimate)
        {
            self.paragraphs.insert(id, info.id);
            self.blocks.insert(info.id, id);
        }
    }

    fn apply_edits<T: TextBackend>(
        &mut self,
        owner: NodeId,
        model: &dyn TextSurfaceModel,
        measurer: &mut TextHost<T>,
        edits: &[BlockChange],
        line_height: f32,
    ) {
        let Some(document) = self.document else {
            return;
        };
        for edit in edits {
            match *edit {
                BlockChange::Inserted { index, id } => {
                    self.insert_block(owner, model, measurer, index, id, line_height);
                }
                BlockChange::Removed { id } => {
                    if let Some(paragraph) = self.paragraphs.remove(&id) {
                        self.blocks.remove(&paragraph);
                        self.resident.retain(|resident| *resident != paragraph);
                        if self.anchor.is_some_and(|(anchor, _)| anchor == paragraph) {
                            self.anchor = None;
                        }
                        measurer.remove_paragraph(owner, document, paragraph);
                    }
                }
                BlockChange::Text { id } => {
                    if let Some(&paragraph) = self.paragraphs.get(&id) {
                        // Measured again if it is on screen; guessed until then.
                        let estimate = model.estimated_lines(id).max(1) as f32 * line_height;
                        measurer.update_paragraph_estimate(owner, document, paragraph, estimate);
                        if !self.resident.contains(&paragraph) {
                            measurer.update_paragraph_height(owner, document, paragraph, None);
                        }
                    }
                }
                // Paint runs are read again for every painted block.
                BlockChange::Paint { .. } => {}
            }
        }
    }

    /// Where the scroll has to be for `at` to show with `margin` around it.
    fn reveal_scroll<T: TextBackend>(
        &self,
        measurer: &TextHost<T>,
        owner: NodeId,
        document: TextDocumentId,
        at: SurfacePosition,
        margin: f32,
        shaped: &FxHashMap<ParagraphId, (crate::text::TextLayoutHandle, SurfaceBlock)>,
    ) -> Reveal {
        let Some(&paragraph) = self.paragraphs.get(&at.block) else {
            return Reveal::Gone;
        };
        let Some(top) = measurer.paragraph_top(owner, document, paragraph) else {
            return Reveal::Gone;
        };
        let rect = shaped.get(&paragraph).and_then(|(handle, _)| {
            measurer
                .query(*handle)?
                .caret_rect_at_byte(at.offset, at.affinity)
        });
        let Some(mut rect) = rect else {
            // Not laid out yet: go there, and place it exactly next pass.
            return Reveal::Jump((top - margin).max(0.0));
        };
        rect.y += top;
        let mut scroll = self.scroll;
        let viewport = self.viewport;
        if rect.y - margin < scroll.y {
            scroll.y = rect.y - margin;
        } else if rect.y + rect.height + margin > scroll.y + viewport.height {
            scroll.y = rect.y + rect.height + margin - viewport.height;
        }
        if self.wrap == SurfaceWrap::None {
            if rect.x - margin < scroll.x {
                scroll.x = rect.x - margin;
            } else if rect.x + rect.width.max(1.0) + margin > scroll.x + viewport.width {
                scroll.x = rect.x + rect.width.max(1.0) + margin - viewport.width;
            }
        }
        scroll.x = scroll.x.max(0.0);
        scroll.y = scroll.y.max(0.0);
        Reveal::Done(scroll)
    }

    /// The model's decorations as rectangles, for the blocks painted, and its
    /// carets, by the index of the painted block they are in.
    fn resolve_decorations<T: TextBackend>(
        &self,
        measurer: &TextHost<T>,
        owner: NodeId,
        document: TextDocumentId,
        decorations: &[SurfaceDecor],
        text_style: &ComputedTextStyle,
    ) -> (Vec<DecorRect>, Vec<(usize, TextCaret)>) {
        let mut rects = Vec::new();
        let mut carets = Vec::new();
        let thickness = (text_style.font_size / 16.0).max(1.0);
        let index_of = |position: &SurfacePosition| {
            let paragraph = *self.paragraphs.get(&position.block)?;
            measurer.paragraph_index(owner, document, paragraph)
        };
        for decor in decorations {
            let (Some(start_index), Some(end_index)) =
                (index_of(&decor.range.start), index_of(&decor.range.end))
            else {
                continue;
            };
            let (start, end, start_index, end_index) =
                if (start_index, decor.range.start.offset) <= (end_index, decor.range.end.offset) {
                    (decor.range.start, decor.range.end, start_index, end_index)
                } else {
                    (decor.range.end, decor.range.start, end_index, start_index)
                };
            for (position, block) in self.painted.iter().enumerate() {
                let Some(index) = measurer.paragraph_index(owner, document, block.paragraph) else {
                    continue;
                };
                if index < start_index || index > end_index {
                    continue;
                }
                let Some(query) =
                    measurer.active_query(owner, TextUnitRef::Paragraph(document, block.paragraph))
                else {
                    continue;
                };
                let local_start = if index == start_index {
                    start.offset
                } else {
                    0
                };
                let local_end = if index == end_index {
                    end.offset
                } else {
                    usize::MAX
                };
                let shift = |mut rect: Rect| {
                    rect.y += block.top;
                    rect
                };
                match decor.kind {
                    DecorKind::Caret { color, width } => {
                        if index == end_index {
                            carets.push((
                                position,
                                TextCaret {
                                    char_index: char_index(block.text.as_str(), end.offset),
                                    color,
                                    width: width.max(1.0),
                                },
                            ));
                        }
                    }
                    DecorKind::Background(color) => {
                        for rect in byte_rects(query, local_start, local_end) {
                            rects.push(DecorRect {
                                rect: shift(rect),
                                color,
                                layer: decor.layer,
                            });
                        }
                    }
                    DecorKind::Outline(color) => {
                        for rect in byte_rects(query, local_start, local_end) {
                            for side in outline(shift(rect), thickness) {
                                rects.push(DecorRect {
                                    rect: side,
                                    color,
                                    layer: decor.layer,
                                });
                            }
                        }
                    }
                    DecorKind::Underline { color, style } => {
                        for rect in byte_rects(query, local_start, local_end) {
                            for piece in underline(shift(rect), thickness, style) {
                                rects.push(DecorRect {
                                    rect: piece,
                                    color,
                                    layer: decor.layer,
                                });
                            }
                        }
                    }
                }
            }
        }
        (rects, carets)
    }

    fn content_point(&self, cx: &EventContext<'_>, position: Point) -> Point {
        let content = content_box(cx);
        Point::new(
            position.x - content.x() + self.scroll.x,
            position.y - content.y() + self.scroll.y,
        )
    }

    /// Runs the model on `input`, then does what it asked.
    fn run_model(&mut self, input: SurfaceInput<'_>, cx: &mut EventContext<'_>) -> EventResult {
        let model_rc = self.handle.model();
        let mut requests = SurfaceRequests::default();
        let result = {
            let geometry = SurfaceGeometry {
                owner: cx.node_id(),
                document: self.document,
                lookup: cx.text_units(),
                paragraphs: &self.paragraphs,
                blocks: &self.blocks,
                scroll: self.scroll,
                viewport: self.viewport,
            };
            let mut surface_cx = SurfaceCx {
                event: cx,
                geometry,
                requests: &mut requests,
            };
            model_rc.borrow_mut().handle_input(input, &mut surface_cx)
        };
        let mut sync = requests.changed;
        if let Some(y) = requests.scroll_to {
            self.scroll.y = y.max(0.0);
            self.anchor = None;
            sync = true;
        }
        if let Some(reveal) = requests.reveal {
            self.reveal = Some(reveal);
            sync = true;
        }
        if sync {
            cx.mark_needs_text_shape();
            cx.mark_needs_paint();
        }
        result
    }

    /// Scrolls by `delta` content pixels, as far as the content allows.
    /// Whether anything moved.
    fn scroll_by(&mut self, dx: f32, dy: f32, cx: &mut EventContext<'_>) -> bool {
        let max_y = (self.document_height - self.viewport.height).max(0.0);
        let max_x = match self.wrap {
            SurfaceWrap::Width => 0.0,
            SurfaceWrap::None => (self.content_width - self.viewport.width).max(0.0),
        };
        let next = Point::new(
            (self.scroll.x + dx).clamp(0.0, max_x),
            (self.scroll.y + dy).clamp(0.0, max_y),
        );
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        self.anchor = None;
        cx.mark_needs_text_shape();
        cx.mark_needs_paint();
        true
    }

    fn click_count(&mut self, at: Instant, point: Point) -> u8 {
        let count = match self.last_click {
            Some((time, last, count))
                if at.saturating_duration_since(time) <= MULTI_CLICK_INTERVAL
                    && last.distance_to(point) <= MULTI_CLICK_DISTANCE =>
            {
                count.saturating_add(1)
            }
            _ => 1,
        };
        self.last_click = Some((at, point, count));
        count
    }

    fn pointer(
        &mut self,
        kind: SurfacePointerKind,
        button: Option<PointerButton>,
        position: Point,
        modifiers: xui_interface::Modifiers,
        click_count: u8,
        cx: &mut EventContext<'_>,
    ) -> EventResult {
        let point = self.content_point(cx, position);
        let geometry = SurfaceGeometry {
            owner: cx.node_id(),
            document: self.document,
            lookup: cx.text_units(),
            paragraphs: &self.paragraphs,
            blocks: &self.blocks,
            scroll: self.scroll,
            viewport: self.viewport,
        };
        let at = geometry.hit_test(point);
        self.run_model(
            SurfaceInput::Pointer(SurfacePointer {
                kind,
                button,
                at,
                point,
                modifiers,
                click_count,
            }),
            cx,
        )
    }

    fn handle_input(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult {
        if self.disabled
            && !matches!(
                event,
                EventRef::Semantic(SemanticEvent::Blur(_) | SemanticEvent::FocusOut(_))
            )
        {
            return EventResult::Ignored;
        }
        match event {
            EventRef::Raw(RawEvent::PointerDown(pointer)) => {
                let point = self.content_point(cx, pointer.position);
                let count = self.click_count(pointer.timestamp, point);
                if pointer.button == PointerButton::Primary {
                    self.drag = Some(pointer.pointer_id);
                    cx.capture_pointer();
                }
                self.pointer(
                    SurfacePointerKind::Down,
                    Some(pointer.button),
                    pointer.position,
                    pointer.modifiers,
                    count,
                    cx,
                );
                EventResult::Consumed
            }
            EventRef::Raw(RawEvent::PointerMove(pointer))
                if self.drag == Some(pointer.pointer_id) =>
            {
                // Dragging past an edge scrolls toward it.
                let content = content_box(cx);
                let dy = if pointer.position.y < content.y() {
                    pointer.position.y - content.y()
                } else if pointer.position.y > content.y() + content.height() {
                    pointer.position.y - content.y() - content.height()
                } else {
                    0.0
                };
                if dy != 0.0 {
                    self.scroll_by(0.0, dy, cx);
                }
                self.pointer(
                    SurfacePointerKind::Drag,
                    pointer.button,
                    pointer.position,
                    pointer.modifiers,
                    0,
                    cx,
                )
            }
            EventRef::Raw(RawEvent::PointerUp(pointer))
                if self.drag == Some(pointer.pointer_id) =>
            {
                self.drag = None;
                cx.release_pointer_capture();
                self.pointer(
                    SurfacePointerKind::Up,
                    Some(pointer.button),
                    pointer.position,
                    pointer.modifiers,
                    0,
                    cx,
                );
                EventResult::Consumed
            }
            EventRef::Raw(RawEvent::PointerCancel(pointer))
                if self.drag == Some(pointer.pointer_id) =>
            {
                self.drag = None;
                cx.release_pointer_capture();
                EventResult::Consumed
            }
            EventRef::Raw(RawEvent::Wheel(wheel)) => {
                let line = self
                    .basis
                    .as_ref()
                    .map_or(16.0, |basis| line_height_px(&basis.style));
                let (dx, dy) = match wheel.delta {
                    xui_interface::events::ScrollDelta::Pixels(delta) => (delta.x, delta.y),
                    xui_interface::events::ScrollDelta::Lines(delta) => {
                        (delta.x * line * 3.0, delta.y * line * 3.0)
                    }
                    xui_interface::events::ScrollDelta::Pages(delta) => (
                        delta.x * self.viewport.width,
                        delta.y * self.viewport.height,
                    ),
                };
                // A positive delta moves the content down, toward the start.
                if self.scroll_by(-dx, -dy, cx) {
                    EventResult::Consumed
                } else {
                    EventResult::Ignored
                }
            }
            EventRef::Raw(RawEvent::Keyboard(key)) => {
                let typed = key.state == KeyState::Down
                    && !key.modifiers.ctrl
                    && !key.modifiers.meta
                    && key
                        .text
                        .as_ref()
                        .is_some_and(|text| !text.as_str().chars().any(char::is_control));
                match key.text.as_ref().filter(|_| typed) {
                    Some(text) => {
                        let text = text.as_str().to_owned();
                        self.run_model(SurfaceInput::Text(&text), cx)
                    }
                    None => self.run_model(SurfaceInput::Key(key), cx),
                }
            }
            EventRef::Raw(RawEvent::Ime(ime)) => {
                self.run_model(SurfaceInput::Ime(ime), cx);
                EventResult::Consumed
            }
            EventRef::Semantic(SemanticEvent::Command(command)) => {
                self.run_model(SurfaceInput::Command(command.command), cx)
            }
            EventRef::Semantic(SemanticEvent::PlatformReply(reply)) => {
                self.run_model(SurfaceInput::PlatformReply(reply.request, &reply.reply), cx)
            }
            EventRef::Semantic(SemanticEvent::Focus(_) | SemanticEvent::FocusIn(_)) => {
                if !self.focused {
                    self.focused = true;
                    cx.mark_needs_paint();
                    self.run_model(SurfaceInput::Focus(true), cx);
                }
                EventResult::Ignored
            }
            EventRef::Semantic(SemanticEvent::Blur(_) | SemanticEvent::FocusOut(_)) => {
                if self.focused {
                    self.focused = false;
                    if self.drag.take().is_some() {
                        cx.release_pointer_capture();
                    }
                    cx.mark_needs_paint();
                    self.run_model(SurfaceInput::Focus(false), cx);
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn has_caret(&self) -> bool {
        self.painted.iter().any(|block| !block.carets.is_empty())
    }

    /// Shows the caret and starts its blink over from `at`, as a text input
    /// does: held still while it is typed into, blinking once left alone.
    fn restart_blink(&mut self, cx: &mut EventContext<'_>, at: Instant) {
        if !self.caret_on {
            self.caret_on = true;
            cx.mark_needs_paint();
        }
        if cx
            .host_capabilities()
            .contains(xui_interface::HostCapabilities::TIMED_REDRAW)
        {
            cx.request_wake_at(at + CARET_BLINK_INTERVAL);
        }
    }

    fn blink(&mut self, wake: &xui_interface::WakeEvent, cx: &mut EventContext<'_>) -> EventResult {
        if !self.focused {
            return EventResult::Ignored;
        }
        self.caret_on = !self.caret_on;
        if self.has_caret() {
            cx.mark_needs_paint();
        }
        let mut next = wake.deadline + CARET_BLINK_INTERVAL;
        if next <= wake.meta.timestamp {
            next = wake.meta.timestamp + CARET_BLINK_INTERVAL;
        }
        cx.request_wake_at(next);
        EventResult::Consumed
    }
}

enum Reveal {
    /// Laid out: the scroll that shows it.
    Done(Point),
    /// Not laid out: a scroll that brings it into range.
    Jump(f32),
    /// Its block is gone.
    Gone,
}

impl HandleEvent for TextSurfaceState {
    fn handle_event(&mut self, event: EventRef<'_>, cx: &mut EventContext<'_>) -> EventResult {
        if let EventRef::Semantic(SemanticEvent::Wake(wake)) = event {
            return self.blink(wake, cx);
        }
        let was_focused = self.focused;
        let result = self.handle_input(event, cx);
        if self.focused {
            if !was_focused || result.is_consumed() {
                let at = match event {
                    EventRef::Raw(raw) => raw.timestamp(),
                    EventRef::Semantic(semantic) => semantic.meta().timestamp,
                };
                self.restart_blink(cx, at);
            }
        } else if was_focused {
            self.caret_on = true;
            cx.cancel_wake();
        }
        result
    }

    /// Reads key, pointer, wheel and IME input raw.
    fn raw_events(&self) -> bool {
        true
    }
}

impl PaintContent for TextSurfaceState {
    fn paint(
        &self,
        node_id: NodeId,
        rect: Bounds,
        style: &ComputedStyle,
        writer: &mut RenderTreeWriter<'_>,
    ) {
        let content = text_input_content_box(rect, style.layout.padding);
        let show_carets = self.focused && self.caret_on;
        let decor = |writer: &mut RenderTreeWriter<'_>, layer: DecorLayer| {
            for decor in &self.decor {
                if decor.layer != layer {
                    continue;
                }
                let rect = decor.rect;
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    continue;
                }
                writer.primitive(Primitive::Shape(ShapePrimitive {
                    bounds: Bounds::from_origin_size((rect.x, rect.y), (rect.width, rect.height)),
                    shape: Shape::Rect,
                    fill: Some(ComputedColorStyle::Solid(decor.color)),
                    stroke: None,
                    shadow: None,
                }))?;
            }
            Ok(())
        };
        writer
            .clip(ClipShape::Rect(content), |writer| {
                let origin =
                    Affine::translate(content.x() - self.scroll.x, content.y() - self.scroll.y);
                writer.transform(origin, |writer| {
                    decor(writer, DecorLayer::BelowText)?;
                    for block in &self.painted {
                        writer.primitive(Primitive::Text(TextPrimitive {
                            node_id,
                            bounds: Bounds::from_origin_size((0.0, block.top), block.size),
                            unit: TextUnitRef::Paragraph(block.document, block.paragraph),
                            layout_revision: block.revision,
                            vertical_align: TextVerticalAlign::Top,
                            paint: if show_carets && !block.carets.is_empty() {
                                let mut paint = block.paint.clone();
                                paint.carets.clone_from(&block.carets);
                                paint
                            } else {
                                block.paint.clone()
                            },
                        }))?;
                    }
                    decor(writer, DecorLayer::AboveText)
                })?;
                Ok(())
            })
            .expect("widget render tree must remain valid");
    }
}

impl PlatformTextInput for TextSurfaceState {
    fn text_input_session(
        &self,
        content_rect: Bounds,
        _text_layout: Option<&dyn TextLayoutQuery>,
    ) -> Option<TextInputSession> {
        if self.disabled {
            return None;
        }
        let caret = self
            .ime_caret
            .unwrap_or_else(|| Rect::new(self.scroll.x, self.scroll.y, 1.0, 16.0));
        Some(TextInputSession {
            cursor_area: Rect::new(
                content_rect.x() + caret.x - self.scroll.x,
                content_rect.y() + caret.y - self.scroll.y,
                caret.width.max(1.0),
                caret.height.max(1.0),
            ),
            purpose: TextInputPurpose::Normal,
            multiline: true,
        })
    }
}

/// The surface's content area in window coordinates.
fn content_box(cx: &EventContext<'_>) -> Bounds {
    let layout = cx.node_ref.layout;
    text_input_content_box(
        Bounds::from_origin_size(cx.node_ref.world_origin, (layout.width(), layout.height())),
        cx.node_ref.effective_style.layout.padding,
    )
}

fn line_height_px(style: &ComputedTextStyle) -> f32 {
    match style.line_height {
        // What a line of the default face takes, near enough for estimates;
        // measured heights replace it.
        LineHeight::Normal => style.font_size * 1.25,
        LineHeight::Px(px) => px,
        LineHeight::Em(em) => em * style.font_size,
    }
    .max(1.0)
}

/// The indices of the blocks overlapping `top..bottom`.
fn visible_range<T: TextBackend>(
    measurer: &TextHost<T>,
    owner: NodeId,
    document: TextDocumentId,
    top: f32,
    bottom: f32,
) -> std::ops::Range<usize> {
    let len = measurer.document_len(owner, document).unwrap_or(0);
    let height = measurer.document_height(owner, document).unwrap_or(0.0);
    if len == 0 {
        return 0..0;
    }
    let index_at = |y: f32| {
        measurer
            .paragraph_at_y(owner, document, y)
            .and_then(|info| measurer.paragraph_index(owner, document, info.id))
    };
    let first = if top <= 0.0 {
        0
    } else {
        index_at(top).unwrap_or(len)
    };
    let last = if bottom >= height {
        len
    } else {
        index_at(bottom).map_or(len, |index| index + 1)
    };
    first.min(len)..last.max(first).min(len)
}

/// The block holding `y`, and how far into it `y` is. `hint` is preferred
/// while `y` is still inside it.
fn anchor_at<T: TextBackend>(
    measurer: &TextHost<T>,
    owner: NodeId,
    document: TextDocumentId,
    y: f32,
    hint: Option<ParagraphId>,
) -> Option<(ParagraphId, f32)> {
    if let Some(paragraph) = hint
        && let Some(info) = measurer.paragraph(owner, document, paragraph)
        && let Some(top) = measurer.paragraph_top(owner, document, paragraph)
        && (top..top + info.height()).contains(&y)
    {
        return Some((paragraph, y - top));
    }
    let info = measurer.paragraph_at_y(owner, document, y)?;
    let top = measurer.paragraph_top(owner, document, info.id)?;
    Some((info.id, y - top))
}

/// How many chars of `text` come before byte `offset`, the unit carets take.
fn char_index(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].chars().count()
}

fn handle_revision(handle: crate::text::TextLayoutHandle) -> u64 {
    use slotmap::Key;
    handle.data().as_ffi()
}

/// The boxes of `start..end` in a laid-out block. `end` past the text is
/// clamped. An empty block still gets a box one caret wide, so a selection
/// across it shows.
fn byte_rects(query: &dyn TextLayoutQuery, start: usize, end: usize) -> Vec<Rect> {
    let rects = query.selection_rects(TextRange::new(
        TextOffset::byte_offset(start.min(end)),
        TextOffset::byte_offset(start.max(end)),
    ));
    if rects.is_empty() && start != end {
        if let Some(mut caret) = query.caret_rect_at_byte(start.min(end), Affinity::After) {
            caret.width = caret.width.max(4.0);
            return vec![caret];
        }
    }
    rects
}

fn outline(rect: Rect, thickness: f32) -> [Rect; 4] {
    [
        Rect::new(rect.x, rect.y, rect.width, thickness),
        Rect::new(
            rect.x,
            rect.y + rect.height - thickness,
            rect.width,
            thickness,
        ),
        Rect::new(rect.x, rect.y, thickness, rect.height),
        Rect::new(
            rect.x + rect.width - thickness,
            rect.y,
            thickness,
            rect.height,
        ),
    ]
}

/// The pieces of an underline under `rect`: one bar, dashes, or a zigzag of
/// short steps for a wavy line.
fn underline(rect: Rect, thickness: f32, style: UnderlineStyle) -> Vec<Rect> {
    let y = rect.y + rect.height - thickness;
    match style {
        UnderlineStyle::Solid => vec![Rect::new(rect.x, y, rect.width, thickness)],
        UnderlineStyle::Dotted => {
            let step = thickness * 2.0;
            let mut x = rect.x;
            let mut pieces = Vec::new();
            while x < rect.x + rect.width {
                pieces.push(Rect::new(
                    x,
                    y,
                    thickness.min(rect.x + rect.width - x),
                    thickness,
                ));
                x += step;
            }
            pieces
        }
        UnderlineStyle::Wavy => {
            let step = thickness * 2.0;
            let mut x = rect.x;
            let mut up = false;
            let mut pieces = Vec::new();
            while x < rect.x + rect.width {
                let piece_y = if up { y - thickness } else { y };
                pieces.push(Rect::new(
                    x,
                    piece_y,
                    step.min(rect.x + rect.width - x),
                    thickness,
                ));
                x += step;
                up = !up;
            }
            pieces
        }
    }
}
