//! What a [`TextSurface`](super::TextSurface) shows and how it is edited: the
//! model trait an editor crate implements, the handle a surface holds it by,
//! and the input and context the model handles events with.

use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};

use rustc_hash::FxHashMap;
use xui_interface::events::{PointerButton, RawIme, RawKeyboard};
use xui_interface::{
    Affinity, Color, CommandId, EventResult, Modifiers, NodeId, ParagraphStyle, PlatformCommand,
    PlatformReply, PlatformRequest, Point, Rect, RequestId, Size, TextContent, TextRun, TextRuns,
    WidgetUpdateFlags,
};

use crate::event_system::EventContext;
use crate::text::{ParagraphId, TextDocumentId, TextUnitLookup, TextUnitRef};
use crate::widgets::CanvasInvalidator;

/// A block's identity, chosen by the model and stable across edits: an
/// insertion above a block does not change its id, so what is on screen is
/// not shaped again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u64);

/// One block: a paragraph of text the surface lays out on its own.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SurfaceBlock {
    /// The paragraph, without a trailing line break.
    pub text: TextContent,
    /// Styles over byte ranges of `text`, on top of the surface's computed
    /// text style. Color and decoration only repaint; font changes reshape.
    pub runs: TextRuns<TextRun>,
    pub paragraph: ParagraphStyle,
}

/// What changed in a model since the surface last asked.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SurfaceChanges {
    #[default]
    None,
    /// Read everything again: the block list, every block.
    Reset,
    /// These edits, in the order they were made.
    Edits(Vec<BlockChange>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockChange {
    /// A block now at `index` of the list.
    Inserted {
        index: usize,
        id: BlockId,
    },
    Removed {
        id: BlockId,
    },
    /// The block's text, or its runs' fonts: it is shaped again.
    Text {
        id: BlockId,
    },
    /// Only its paint runs: it is drawn again, not shaped.
    Paint {
        id: BlockId,
    },
}

/// A place in the text: a byte offset into one block's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfacePosition {
    pub block: BlockId,
    pub offset: usize,
    /// Which line a position at a soft line break belongs to.
    pub affinity: Affinity,
}

impl SurfacePosition {
    pub fn new(block: BlockId, offset: usize) -> Self {
        Self {
            block,
            offset,
            affinity: Affinity::After,
        }
    }
}

/// From `start` to `end`, possibly across blocks. Either may come first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceRange {
    pub start: SurfacePosition,
    pub end: SurfacePosition,
}

impl SurfaceRange {
    pub fn new(start: SurfacePosition, end: SurfacePosition) -> Self {
        Self { start, end }
    }

    pub fn caret(at: SurfacePosition) -> Self {
        Self { start: at, end: at }
    }
}

/// Something drawn over or under the text that is not glyph paint: a
/// selection, a caret, the current line, a find match, a diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceDecor {
    pub range: SurfaceRange,
    pub kind: DecorKind,
    pub layer: DecorLayer,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DecorKind {
    /// Fills the range's boxes.
    Background(Color),
    /// A line under the range's boxes.
    Underline { color: Color, style: UnderlineStyle },
    /// A box around each of the range's line boxes.
    Outline(Color),
    /// A caret at the range's `end`. It blinks while the surface is focused.
    Caret { color: Color, width: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnderlineStyle {
    #[default]
    Solid,
    Dotted,
    Wavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecorLayer {
    #[default]
    BelowText,
    AboveText,
}

/// An event, as the model receives it: with pointer positions resolved to
/// text positions, which only the surface can do.
#[derive(Debug, Clone, Copy)]
pub enum SurfaceInput<'a> {
    Pointer(SurfacePointer),
    Key(&'a RawKeyboard),
    /// Text a key typed, outside an IME composition. Line breaks and tabs
    /// arrive as keys, not here.
    Text(&'a str),
    Ime(&'a RawIme),
    /// A standard command, from a menu or a binding: copy, paste, undo, ...
    Command(CommandId),
    /// The answer to a [`SurfaceCx::request_platform`].
    PlatformReply(RequestId, &'a PlatformReply),
    Focus(bool),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePointer {
    pub kind: SurfacePointerKind,
    pub button: Option<PointerButton>,
    /// Where it is in the text; `None` for a document with no blocks.
    pub at: Option<SurfacePosition>,
    /// In the surface's content coordinates: document y, scroll applied.
    pub point: Point,
    pub modifiers: Modifiers,
    /// 1 for a single click, 2 for a double, ... Only on `Down`.
    pub click_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfacePointerKind {
    Down,
    /// Only while a button that went down on the surface is held.
    Drag,
    Up,
}

/// A text surface's content and behavior, implemented by an editor crate.
///
/// The surface reads it while it lays out and paints -- `&self`, apart from
/// [`take_changes`](Self::take_changes) -- and calls
/// [`handle_input`](Self::handle_input) only from event dispatch. Neither may
/// write a reactive signal: a model reaches the rest of the application
/// through its own channels.
pub trait TextSurfaceModel {
    /// What changed since the last call. The surface calls it once per sync,
    /// and treats the first call after binding as a `Reset` whatever it says.
    fn take_changes(&mut self) -> SurfaceChanges;

    fn block_count(&self) -> usize;
    fn block_at(&self, index: usize) -> BlockId;
    fn block(&self, id: BlockId) -> SurfaceBlock;

    /// How many lines a block not yet laid out is guessed to take, for the
    /// scroll extent. Exact heights replace the guesses as blocks are shown.
    fn estimated_lines(&self, _id: BlockId) -> u32 {
        1
    }

    /// What to draw around the text of `visible` blocks, in order.
    fn decorations(&self, _visible: &[BlockId]) -> Vec<SurfaceDecor> {
        Vec::new()
    }

    /// The caret the platform's IME candidate window follows.
    fn primary_caret(&self) -> Option<SurfacePosition> {
        None
    }

    fn handle_input(
        &mut self,
        _input: SurfaceInput<'_>,
        _cx: &mut SurfaceCx<'_, '_>,
    ) -> EventResult {
        EventResult::Ignored
    }
}

/// The model a surface starts with: no blocks.
struct EmptyModel;

impl TextSurfaceModel for EmptyModel {
    fn take_changes(&mut self) -> SurfaceChanges {
        SurfaceChanges::None
    }

    fn block_count(&self) -> usize {
        0
    }

    fn block_at(&self, _index: usize) -> BlockId {
        BlockId(0)
    }

    fn block(&self, _id: BlockId) -> SurfaceBlock {
        SurfaceBlock::default()
    }
}

/// A [`TextSurfaceModel`] shared with the surfaces showing it.
///
/// Changing the model does nothing on screen by itself: call
/// [`invalidate`](Self::invalidate) after, and every surface showing it syncs
/// on the next frame, with no component rebuild. Compared by identity, like
/// `CanvasController`: hold it in component state rather than building one
/// each render. Do not keep the model borrowed across a frame; the surface
/// borrows it to lay out and to handle events.
#[derive(Clone)]
pub struct TextSurfaceHandle {
    inner: Rc<HandleInner>,
}

struct HandleInner {
    model: Rc<RefCell<dyn TextSurfaceModel>>,
    bindings: RefCell<Vec<NodeId>>,
    invalidator: RefCell<Option<CanvasInvalidator>>,
}

impl TextSurfaceHandle {
    pub fn new<M: TextSurfaceModel + 'static>(model: Rc<RefCell<M>>) -> Self {
        let model: Rc<RefCell<dyn TextSurfaceModel>> = model;
        Self {
            inner: Rc::new(HandleInner {
                model,
                bindings: RefCell::new(Vec::new()),
                invalidator: RefCell::new(None),
            }),
        }
    }

    /// A handle to a model with no blocks.
    pub fn empty() -> Self {
        Self::new(Rc::new(RefCell::new(EmptyModel)))
    }

    /// Syncs every surface showing the model on the next frame.
    pub fn invalidate(&self) {
        let invalidator = self.inner.invalidator.borrow().clone();
        let Some(invalidator) = invalidator else {
            return;
        };
        for id in self.inner.bindings.borrow().iter() {
            invalidator.push(
                *id,
                WidgetUpdateFlags::PAINT_OUTPUT | WidgetUpdateFlags::TEXT_SHAPE,
            );
        }
    }

    /// A handle that does not keep the model alive, for whatever the model
    /// itself refers back to (a shared buffer notifying its views).
    pub fn downgrade(&self) -> WeakTextSurfaceHandle {
        WeakTextSurfaceHandle {
            inner: Rc::downgrade(&self.inner),
        }
    }

    pub fn same_handle(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn model(&self) -> Rc<RefCell<dyn TextSurfaceModel>> {
        self.inner.model.clone()
    }

    pub(crate) fn bind(&self, id: NodeId, invalidator: CanvasInvalidator) {
        let mut bindings = self.inner.bindings.borrow_mut();
        if !bindings.contains(&id) {
            bindings.push(id);
        }
        *self.inner.invalidator.borrow_mut() = Some(invalidator);
    }

    pub(crate) fn unbind(&self, id: NodeId) {
        let mut bindings = self.inner.bindings.borrow_mut();
        bindings.retain(|bound| *bound != id);
        if bindings.is_empty() {
            *self.inner.invalidator.borrow_mut() = None;
        }
    }
}

/// A [`TextSurfaceHandle`] that does not keep its model alive.
#[derive(Clone)]
pub struct WeakTextSurfaceHandle {
    inner: Weak<HandleInner>,
}

impl WeakTextSurfaceHandle {
    /// The handle, while anything still holds it.
    pub fn upgrade(&self) -> Option<TextSurfaceHandle> {
        self.inner.upgrade().map(|inner| TextSurfaceHandle { inner })
    }
}

impl fmt::Debug for WeakTextSurfaceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WeakTextSurfaceHandle")
            .field("alive", &(self.inner.strong_count() > 0))
            .finish()
    }
}

impl PartialEq for TextSurfaceHandle {
    fn eq(&self, other: &Self) -> bool {
        self.same_handle(other)
    }
}

impl fmt::Debug for TextSurfaceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextSurfaceHandle")
            .field("bindings", &self.inner.bindings.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Default for TextSurfaceHandle {
    fn default() -> Self {
        Self::empty()
    }
}

/// What a model asks the surface to do while it handles an event, applied
/// once the event returns.
#[derive(Debug, Default)]
pub(crate) struct SurfaceRequests {
    pub scroll_to: Option<f32>,
    pub reveal: Option<(SurfacePosition, f32)>,
    pub changed: bool,
}

/// The surface's geometry and the platform, for a model handling an event.
///
/// Geometry answers only for blocks the surface has laid out: the visible ones
/// and at least a viewport's worth on each side. A position anywhere else gets
/// `None`; jumping there is offset arithmetic in the model, then
/// [`reveal`](Self::reveal).
pub struct SurfaceCx<'a, 'e> {
    pub(crate) event: &'a mut EventContext<'e>,
    pub(crate) geometry: SurfaceGeometry<'a>,
    pub(crate) requests: &'a mut SurfaceRequests,
}

/// What the surface knows of its own layout, borrowed for one event.
#[derive(Clone, Copy)]
pub(crate) struct SurfaceGeometry<'a> {
    pub owner: NodeId,
    pub document: Option<TextDocumentId>,
    pub lookup: Option<&'a dyn TextUnitLookup>,
    pub paragraphs: &'a FxHashMap<BlockId, ParagraphId>,
    pub blocks: &'a FxHashMap<ParagraphId, BlockId>,
    pub scroll: Point,
    pub viewport: Size<f32>,
}

impl<'a> SurfaceGeometry<'a> {
    fn query(&self, block: BlockId) -> Option<(ParagraphId, &'a dyn crate::text::TextLayoutQuery)> {
        let document = self.document?;
        let paragraph = *self.paragraphs.get(&block)?;
        let query = self
            .lookup?
            .active_query(self.owner, TextUnitRef::Paragraph(document, paragraph))?;
        Some((paragraph, query))
    }

    pub(crate) fn top(&self, paragraph: ParagraphId) -> Option<f32> {
        self.lookup?
            .paragraph_top(self.owner, self.document?, paragraph)
    }

    /// The position at `point`, in content coordinates.
    pub(crate) fn hit_test(&self, point: Point) -> Option<SurfacePosition> {
        let (lookup, document) = (self.lookup?, self.document?);
        let len = lookup.document_len(self.owner, document)?;
        if len == 0 {
            return None;
        }
        let height = lookup.document_height(self.owner, document)?;
        let paragraph = if point.y < 0.0 {
            lookup.paragraph_at(self.owner, document, 0)?
        } else if point.y >= height {
            lookup.paragraph_at(self.owner, document, len - 1)?
        } else {
            lookup.paragraph_at_y(self.owner, document, point.y)?
        };
        let block = *self.blocks.get(&paragraph)?;
        let (_, query) = self.query(block)?;
        let top = self.top(paragraph)?;
        let local = Point::new(
            point.x,
            (point.y - top).clamp(0.0, query.size().height.max(0.0)),
        );
        let (offset, affinity) = query.hit_test_byte(local)?;
        Some(SurfacePosition {
            block,
            offset,
            affinity,
        })
    }

    /// The caret box at `at`, in content coordinates.
    pub(crate) fn caret_rect(&self, at: SurfacePosition) -> Option<Rect> {
        let (paragraph, query) = self.query(at.block)?;
        let top = self.top(paragraph)?;
        let mut rect = query.caret_rect_at_byte(at.offset, at.affinity)?;
        rect.y += top;
        Some(rect)
    }
}

impl<'a, 'e> SurfaceCx<'a, 'e> {
    /// The position under `point`, in content coordinates (the coordinates
    /// [`SurfacePointer::point`] is in).
    pub fn hit_test(&self, point: Point) -> Option<SurfacePosition> {
        self.geometry.hit_test(point)
    }

    /// The caret box at `at`, in content coordinates.
    pub fn caret_rect(&self, at: SurfacePosition) -> Option<Rect> {
        self.geometry.caret_rect(at)
    }

    /// The boxes covering `start..end` of one block's text, in content
    /// coordinates.
    pub fn selection_rects(&self, block: BlockId, start: usize, end: usize) -> Vec<Rect> {
        let Some((paragraph, query)) = self.geometry.query(block) else {
            return Vec::new();
        };
        let top = self.geometry.top(paragraph).unwrap_or(0.0);
        query
            .selection_rects(xui_interface::TextRange::new(
                xui_interface::TextOffset::byte_offset(start.min(end)),
                xui_interface::TextOffset::byte_offset(start.max(end)),
            ))
            .into_iter()
            .map(|mut rect| {
                rect.y += top;
                rect
            })
            .collect()
    }

    /// The top of `block` in content coordinates, laid out or estimated.
    pub fn block_top(&self, block: BlockId) -> Option<f32> {
        self.geometry.top(*self.geometry.paragraphs.get(&block)?)
    }

    /// The position `lines` lines below `from` (above, when negative), as
    /// close to `goal_x` as the line allows. Crosses blocks; stops at the
    /// first and last line of the document. `None` when `from` is not laid
    /// out.
    pub fn vertical_move(
        &self,
        from: SurfacePosition,
        goal_x: f32,
        lines: i32,
    ) -> Option<SurfacePosition> {
        let mut at = from;
        for _ in 0..lines.unsigned_abs() {
            let rect = self.geometry.caret_rect(at)?;
            let step = rect.height.max(1.0);
            let y = if lines < 0 {
                rect.y - step * 0.5
            } else {
                rect.y + rect.height + step * 0.5
            };
            let next = self.geometry.hit_test(Point::new(goal_x, y))?;
            if next == at {
                break;
            }
            at = next;
        }
        Some(at)
    }

    /// The visible area's size.
    pub fn viewport(&self) -> Size<f32> {
        self.geometry.viewport
    }

    /// How far the content is scrolled.
    pub fn scroll(&self) -> Point {
        self.geometry.scroll
    }

    pub fn scroll_to(&mut self, y: f32) {
        self.requests.scroll_to = Some(y);
    }

    /// Scrolls just enough to show `at` with `margin` around it, once it is
    /// laid out. Works for positions far outside the laid-out blocks too.
    pub fn reveal(&mut self, at: SurfacePosition, margin: f32) {
        self.requests.reveal = Some((at, margin));
    }

    /// The model changed: the surface syncs before the next frame. The same
    /// as `TextSurfaceHandle::invalidate`, for the surface handling the event.
    pub fn changed(&mut self) {
        self.requests.changed = true;
    }

    /// See `EventContext::send_platform`.
    pub fn send_platform(&self, command: PlatformCommand) -> bool {
        self.event.send_platform(command)
    }

    /// See `EventContext::request_platform`. The reply comes back as
    /// [`SurfaceInput::PlatformReply`].
    pub fn request_platform(&self, request: PlatformRequest) -> Option<RequestId> {
        self.event.request_platform(request)
    }

    pub fn capture_pointer(&mut self) {
        self.event.capture_pointer();
    }

    pub fn release_pointer_capture(&mut self) {
        self.event.release_pointer_capture();
    }
}
