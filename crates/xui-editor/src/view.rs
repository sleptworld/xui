//! One view of a buffer: its selections, keymap and input state, and how
//! input edits the buffer. It is the [`TextSurfaceModel`] a `TextSurface`
//! shows.
//!
//! A view follows the buffer's log. Before it does anything it maps its
//! selections through the edits other views and the application made since
//! it last looked ([`EditorView::catch_up`]); when its surface syncs it hands
//! over the blocks those edits changed. Its own edits go through a
//! [`Transaction`](crate::Transaction) like everyone else's.

use rustc_hash::FxHashMap;
use xui_core::prelude::{
    BlockId, Color, DecorKind, DecorLayer, ParagraphStyle, SurfaceBlock, SurfaceChanges, SurfaceCx,
    SurfaceDecor, SurfaceInput, SurfacePointer, SurfacePointerKind, SurfacePosition, SurfaceRange,
    TextSurfaceModel, UnderlineStyle, WhiteSpace,
};
use xui_interface::events::{PointerButton, RawIme};
use xui_interface::{
    ClipboardContent, ClipboardTarget, CommandId, EventResult, PlatformCommand, PlatformReply,
    PlatformRequest, Point, RequestId, commands,
};

use crate::buffer::{BufferHandle, LogChange, Origin, Revision, ViewId, shift};
use crate::document::{Document, Pos, end_of, normalize_breaks, ordered};
use crate::history::EditKind;
use crate::keymap::{EditorCommand, Keymap, Motion};
use crate::selection::{Selection, Selections};

/// How a view draws what is not text, and how it edits.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorStyle {
    pub selection: Color,
    pub caret: Color,
    pub caret_width: f32,
    /// The underline under an IME composition.
    pub composition: Color,
    /// What Tab inserts.
    pub indent: String,
    /// Whether lines wrap; the surface's `wrap` prop should agree.
    pub wrap: bool,
}

impl Default for EditorStyle {
    fn default() -> Self {
        Self {
            selection: Color::rgba(0.18, 0.42, 0.88, 0.28),
            caret: Color::rgba(0.1, 0.1, 0.1, 1.0),
            caret_width: 2.0,
            composition: Color::rgba(0.1, 0.1, 0.1, 0.8),
            indent: "    ".to_owned(),
            wrap: true,
        }
    }
}

/// What a view needs from its surface while it edits: geometry for moves
/// that depend on layout, and the platform for the clipboard. The surface's
/// [`SurfaceCx`] is one; a test can supply its own.
pub trait EditorHost {
    /// See `SurfaceCx::vertical_move`.
    fn vertical_move(
        &self,
        from: SurfacePosition,
        goal_x: f32,
        lines: i32,
    ) -> Option<SurfacePosition>;
    /// The x of the caret at `at`, in content coordinates.
    fn caret_x(&self, at: SurfacePosition) -> Option<f32>;
    /// How many lines a page up or down moves.
    fn page_lines(&self) -> i32;
    fn reveal(&mut self, at: SurfacePosition);
    fn changed(&mut self);
    fn write_clipboard(&mut self, text: String) -> bool;
    /// Asks for the clipboard; the reply comes back as
    /// `SurfaceInput::PlatformReply` with this id.
    fn read_clipboard(&mut self) -> Option<RequestId>;
    fn capture_pointer(&mut self);
    fn release_pointer_capture(&mut self);
}

impl EditorHost for SurfaceCx<'_, '_> {
    fn vertical_move(
        &self,
        from: SurfacePosition,
        goal_x: f32,
        lines: i32,
    ) -> Option<SurfacePosition> {
        SurfaceCx::vertical_move(self, from, goal_x, lines)
    }

    fn caret_x(&self, at: SurfacePosition) -> Option<f32> {
        self.caret_rect(at).map(|rect| rect.x)
    }

    fn page_lines(&self) -> i32 {
        let line = self.caret_rect_height().unwrap_or(16.0).max(1.0);
        ((self.viewport().height / line) as i32 - 1).max(1)
    }

    fn reveal(&mut self, at: SurfacePosition) {
        SurfaceCx::reveal(self, at, 16.0);
    }

    fn changed(&mut self) {
        SurfaceCx::changed(self);
    }

    fn write_clipboard(&mut self, text: String) -> bool {
        self.send_platform(PlatformCommand::WriteClipboard {
            target: ClipboardTarget::General,
            content: ClipboardContent::Text(text),
        })
    }

    fn read_clipboard(&mut self) -> Option<RequestId> {
        self.request_platform(PlatformRequest::ReadClipboard(ClipboardTarget::General))
    }

    fn capture_pointer(&mut self) {
        SurfaceCx::capture_pointer(self);
    }

    fn release_pointer_capture(&mut self) {
        SurfaceCx::release_pointer_capture(self);
    }
}

/// A rough line height for paging, from the caret at the top of the view.
trait CaretHeight {
    fn caret_rect_height(&self) -> Option<f32>;
}

impl CaretHeight for SurfaceCx<'_, '_> {
    fn caret_rect_height(&self) -> Option<f32> {
        let scroll = self.scroll();
        let top = self.hit_test(Point::new(scroll.x, scroll.y + 1.0))?;
        self.caret_rect(top).map(|rect| rect.height)
    }
}

/// What a pointer drag extends by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Grapheme,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    /// The range the press selected, which the drag always keeps.
    origin: (Pos, Pos),
    granularity: Granularity,
}

/// An IME composition in progress: its text is in the buffer, unrecorded,
/// from `start` to `end`.
#[derive(Debug, Clone, Copy)]
struct Composition {
    start: Pos,
    end: Pos,
}

/// One view of a buffer: any number of selections, a keymap, an IME
/// composition, and the input handling that edits the buffer. Show it with a
/// `TextSurface` through an [`EditorHandle`](crate::EditorHandle); give two
/// views one [`BufferHandle`] for a split editor.
pub struct EditorView {
    buffer: BufferHandle,
    id: ViewId,
    /// How far the selections have followed the buffer's log.
    selections_seen: Revision,
    /// How far the surface has been told.
    blocks_seen: Revision,
    /// The surface reads everything again on its next sync.
    reset: bool,

    selections: Selections,
    keymap: Keymap,
    style: EditorStyle,
    read_only: bool,

    pending_paste: Option<RequestId>,
    drag: Option<Drag>,
    composition: Option<Composition>,
    on_change: Option<Box<dyn FnMut(&EditorView)>>,
}

impl std::fmt::Debug for EditorView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorView")
            .field("id", &self.id)
            .field("selections", &self.selections)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl Drop for EditorView {
    fn drop(&mut self) {
        // A view dropped while its buffer is borrowed is forgotten later,
        // when the buffer finds its surface gone.
        if let Some(mut buffer) = self.buffer.try_borrow_mut() {
            buffer.unregister_view(self.id);
        }
    }
}

impl EditorView {
    /// A view of `buffer`, with a caret at its start.
    pub fn new(buffer: &BufferHandle) -> Self {
        let (id, revision) = {
            let mut shared = buffer.borrow_mut();
            (shared.register_view(), shared.revision())
        };
        Self {
            buffer: buffer.clone(),
            id,
            selections_seen: revision,
            blocks_seen: revision,
            reset: true,
            selections: Selections::single(Selection::caret(Pos::default())),
            keymap: Keymap::platform_default(),
            style: EditorStyle::default(),
            read_only: false,
            pending_paste: None,
            drag: None,
            composition: None,
            on_change: None,
        }
    }

    pub fn buffer(&self) -> &BufferHandle {
        &self.buffer
    }

    pub fn id(&self) -> ViewId {
        self.id
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// The selections, as of the last time this view followed the log.
    pub fn selections(&self) -> &Selections {
        &self.selections
    }

    pub fn set_selections(&mut self, selections: Selections) {
        self.catch_up();
        let buffer = self.buffer.borrow();
        let doc = buffer.document();
        let list = selections
            .iter()
            .map(|selection| Selection::new(doc.clamp(selection.anchor), doc.clamp(selection.head)))
            .collect();
        drop(buffer);
        self.selections = Selections::from_list(list, selections.primary_index());
        self.buffer.borrow_mut().seal_history();
    }

    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    pub fn style(&self) -> &EditorStyle {
        &self.style
    }

    pub fn set_style(&mut self, style: EditorStyle) {
        if style.wrap != self.style.wrap {
            // Every block's paragraph style changes with it.
            self.reset = true;
        }
        self.style = style;
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Called after every input of this view's that edited the buffer, with
    /// the view as it is after. It runs during event dispatch, so it may
    /// update application state, but not edit the buffer.
    pub fn set_on_change(&mut self, on_change: impl FnMut(&EditorView) + 'static) {
        self.on_change = Some(Box::new(on_change));
    }

    pub fn can_undo(&self) -> bool {
        self.buffer.read(|buffer| buffer.can_undo())
    }

    pub fn can_redo(&self) -> bool {
        self.buffer.read(|buffer| buffer.can_redo())
    }

    /// Maps the selections and any composition through what the buffer's
    /// log holds past where this view last looked.
    pub fn catch_up(&mut self) {
        let buffer = self.buffer.clone();
        let buffer = buffer.borrow();
        match buffer.log_since(self.selections_seen) {
            Some(entries) => {
                for entry in entries {
                    match &entry.change {
                        LogChange::Edit { edit, .. } => {
                            self.selections.map(|selection| Selection {
                                anchor: edit.map(selection.anchor),
                                head: edit.map(selection.head),
                                goal_x: selection.goal_x,
                            });
                            if let Some(composition) = &mut self.composition {
                                composition.start = edit.map(composition.start);
                                composition.end = edit.map(composition.end);
                            }
                        }
                        LogChange::Reset => {
                            self.selections = Selections::single(Selection::caret(Pos::default()));
                            self.composition = None;
                            self.drag = None;
                        }
                        LogChange::Restyle { .. } => {}
                    }
                }
            }
            // Fell behind the log: keep what still fits the text.
            None => self.composition = None,
        }
        let doc = buffer.document();
        self.selections.map(|selection| Selection {
            anchor: doc.clamp(selection.anchor),
            head: doc.clamp(selection.head),
            goal_x: selection.goal_x,
        });
        self.selections_seen = buffer.revision();
    }

    /// Runs `command` as if its key had been pressed.
    pub fn run(&mut self, command: EditorCommand, host: &mut dyn EditorHost) -> EventResult {
        self.catch_up();
        let before = self.revision();
        let result = self.execute(command, host);
        self.finish(before, host);
        result
    }

    /// Takes one event from the surface.
    pub fn handle(&mut self, input: SurfaceInput<'_>, host: &mut dyn EditorHost) -> EventResult {
        self.catch_up();
        let before = self.revision();
        let result = match input {
            SurfaceInput::Pointer(pointer) => self.pointer(pointer, host),
            SurfaceInput::Key(key) => match self.keymap.resolve(key) {
                Some(command) => self.execute(command, host),
                None => EventResult::Ignored,
            },
            SurfaceInput::Text(text) => {
                if self.read_only {
                    return EventResult::Ignored;
                }
                self.end_composition();
                self.insert_all(text, EditKind::Typing);
                EventResult::Consumed
            }
            SurfaceInput::Ime(ime) => self.ime(ime),
            SurfaceInput::Command(command) => match command_for(command) {
                Some(command) => self.execute(command, host),
                None => EventResult::Ignored,
            },
            SurfaceInput::PlatformReply(request, reply) if self.pending_paste == Some(request) => {
                self.pending_paste = None;
                match reply {
                    PlatformReply::Clipboard(Some(ClipboardContent::Text(text)))
                        if !self.read_only =>
                    {
                        self.paste(text);
                        EventResult::Consumed
                    }
                    _ => EventResult::Ignored,
                }
            }
            SurfaceInput::PlatformReply(..) => return EventResult::Ignored,
            SurfaceInput::Focus(false) => {
                self.drag = None;
                return EventResult::Ignored;
            }
            SurfaceInput::Focus(true) => return EventResult::Ignored,
        };
        if result.is_consumed() {
            self.finish(before, host);
        }
        result
    }

    fn revision(&self) -> Revision {
        self.buffer.read(|buffer| buffer.revision())
    }

    /// After any input: tell the surface, follow the caret, tell the app if
    /// the buffer moved.
    fn finish(&mut self, before: Revision, host: &mut dyn EditorHost) {
        host.changed();
        host.reveal(self.surface_position(self.selections.primary().head));
        if self.revision() != before
            && let Some(mut on_change) = self.on_change.take()
        {
            on_change(self);
            self.on_change.get_or_insert(on_change);
        }
    }

    fn execute(&mut self, command: EditorCommand, host: &mut dyn EditorHost) -> EventResult {
        use EditorCommand::*;
        let editing = !matches!(command, Move { .. } | SelectAll | Copy | CollapseSelections);
        if editing && self.read_only {
            return EventResult::Ignored;
        }
        if editing {
            self.end_composition();
        }
        match command {
            Move { motion, extend } => self.move_selections(motion, extend, host),
            Backspace => {
                self.delete_with(EditKind::Deleting, |doc, at| (doc.prev_grapheme(at), at))
            }
            Delete => self.delete_with(EditKind::Deleting, |doc, at| (at, doc.next_grapheme(at))),
            DeleteWordBackward => {
                self.delete_with(EditKind::Other, |doc, at| (doc.prev_word(at), at))
            }
            DeleteWordForward => {
                self.delete_with(EditKind::Other, |doc, at| (at, doc.next_word(at)))
            }
            DeleteToLineStart => {
                self.delete_with(EditKind::Other, |_, at| (Pos::new(at.line, 0), at))
            }
            Newline => {
                let texts: Vec<String> = {
                    let buffer = self.buffer.borrow();
                    let doc = buffer.document();
                    self.selections
                        .iter()
                        .map(|selection| format!("\n{}", doc.indent(selection.start().line)))
                        .collect()
                };
                self.insert_each(texts, EditKind::Other);
            }
            Indent => self.indent(),
            Outdent => self.outdent(),
            Insert(text) => self.insert_all(&text, EditKind::Typing),
            SelectAll => {
                let (start, end) = self
                    .buffer
                    .read(|buffer| (buffer.document().start(), buffer.document().end()));
                self.selections = Selections::single(Selection::new(start, end));
            }
            Copy => {
                let text = self.selected_text();
                if !text.is_empty() {
                    host.write_clipboard(text);
                }
            }
            Cut => {
                let text = self.selected_text();
                if !text.is_empty() && host.write_clipboard(text) {
                    self.delete_with(EditKind::Other, |_, at| (at, at));
                }
            }
            Paste => self.pending_paste = host.read_clipboard(),
            Undo => self.undo_redo(false),
            Redo => self.undo_redo(true),
            CollapseSelections => self.selections.collapse(),
        }
        EventResult::Consumed
    }

    /// Undo and redo are the buffer's: they revert whichever view's step was
    /// last. The view that made it gets its selections back; any other just
    /// follows the edits.
    fn undo_redo(&mut self, redo: bool) {
        let before = self.selections_seen;
        let outcome = {
            let mut buffer = self.buffer.borrow_mut();
            if redo { buffer.redo() } else { buffer.undo() }
        };
        match outcome {
            Some((Some(view), Some(selections))) if view == self.id => {
                self.selections = selections;
                self.selections_seen = self.revision();
                self.catch_up();
            }
            Some(_) => {
                self.selections_seen = before;
                self.catch_up();
            }
            None => {}
        }
    }

    fn pointer(&mut self, pointer: SurfacePointer, host: &mut dyn EditorHost) -> EventResult {
        let Some(at) = pointer.at else {
            return EventResult::Ignored;
        };
        let at = self.pos_of(at);
        match pointer.kind {
            SurfacePointerKind::Down if pointer.button == Some(PointerButton::Primary) => {
                self.end_composition();
                self.buffer.borrow_mut().seal_history();
                let granularity = match pointer.click_count {
                    0 | 1 => Granularity::Grapheme,
                    2 => Granularity::Word,
                    _ => Granularity::Line,
                };
                let origin = self.unit_at(at, granularity);
                if pointer.modifiers.shift {
                    let anchor = self.selections.primary().anchor;
                    let mut list = self.selections.list().to_vec();
                    list[self.selections.primary_index()] = Selection::new(anchor, at);
                    self.selections = Selections::from_list(list, self.selections.primary_index());
                    self.drag = Some(Drag {
                        origin: (anchor, anchor),
                        granularity,
                    });
                } else {
                    let selection = Selection::new(origin.0, origin.1);
                    if pointer.modifiers.alt {
                        self.selections.add(selection);
                    } else {
                        self.selections = Selections::single(selection);
                    }
                    self.drag = Some(Drag {
                        origin,
                        granularity,
                    });
                }
                host.capture_pointer();
                EventResult::Consumed
            }
            SurfacePointerKind::Drag => {
                let Some(drag) = self.drag else {
                    return EventResult::Ignored;
                };
                let unit = self.unit_at(at, drag.granularity);
                let selection = if unit.0 < drag.origin.0 {
                    Selection::new(drag.origin.1, unit.0)
                } else {
                    Selection::new(drag.origin.0, unit.1.max(drag.origin.1))
                };
                let mut list = self.selections.list().to_vec();
                list[self.selections.primary_index()] = selection;
                self.selections = Selections::from_list(list, self.selections.primary_index());
                EventResult::Consumed
            }
            SurfacePointerKind::Up => {
                self.drag = None;
                host.release_pointer_capture();
                EventResult::Consumed
            }
            SurfacePointerKind::Down => EventResult::Ignored,
        }
    }

    /// The grapheme, word or line at `at`, as a range.
    fn unit_at(&self, at: Pos, granularity: Granularity) -> (Pos, Pos) {
        let buffer = self.buffer.borrow();
        let doc = buffer.document();
        match granularity {
            Granularity::Grapheme => (at, at),
            Granularity::Word => doc.word_at(at),
            Granularity::Line => {
                let start = Pos::new(at.line, 0);
                let end = if at.line + 1 < doc.line_count() {
                    Pos::new(at.line + 1, 0)
                } else {
                    Pos::new(at.line, doc.line_len(at.line))
                };
                (start, end)
            }
        }
    }

    fn move_selections(&mut self, motion: Motion, extend: bool, host: &mut dyn EditorHost) {
        self.buffer.borrow_mut().seal_history();
        let vertical = matches!(
            motion,
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown
        );
        let mut list = self.selections.list().to_vec();
        for selection in &mut list {
            // Without extend, a selection collapses toward the move first.
            if !extend && !selection.is_empty() && matches!(motion, Motion::Left | Motion::Right) {
                let to = if motion == Motion::Left {
                    selection.start()
                } else {
                    selection.end()
                };
                *selection = Selection::caret(to);
                continue;
            }
            let head = selection.head;
            let (next, goal_x) = if vertical {
                let lines = match motion {
                    Motion::Up => -1,
                    Motion::Down => 1,
                    Motion::PageUp => -host.page_lines(),
                    _ => host.page_lines(),
                };
                let from = self.surface_position(head);
                let goal = selection
                    .goal_x
                    .or_else(|| host.caret_x(from))
                    .unwrap_or(0.0);
                let next = match host.vertical_move(from, goal, lines) {
                    Some(next) if next != from => self.pos_of(next),
                    // At the first or last line: to its start or end.
                    _ => self.buffer.read(|buffer| {
                        let doc = buffer.document();
                        if lines < 0 { doc.start() } else { doc.end() }
                    }),
                };
                (next, Some(goal))
            } else {
                let buffer = self.buffer.borrow();
                let doc = buffer.document();
                let next = match motion {
                    Motion::Left => doc.prev_grapheme(head),
                    Motion::Right => doc.next_grapheme(head),
                    Motion::WordLeft => doc.prev_word(head),
                    Motion::WordRight => doc.next_word(head),
                    Motion::LineStart => line_start(doc, head),
                    Motion::LineEnd => Pos::new(head.line, doc.line_len(head.line)),
                    Motion::DocumentStart => doc.start(),
                    Motion::DocumentEnd => doc.end(),
                    _ => head,
                };
                (next, None)
            };
            *selection = Selection {
                anchor: if extend { selection.anchor } else { next },
                head: next,
                goal_x,
            };
        }
        self.selections = Selections::from_list(list, self.selections.primary_index());
    }

    /// Replaces each selection, last first, with what `edit` makes of it: a
    /// range and its replacement, as one step. Positions the edits after it
    /// moved are mapped through; the selections end as carets after their
    /// insertions.
    fn edit_each(
        &mut self,
        kind: EditKind,
        mut edit: impl FnMut(&Document, usize, &Selection) -> Option<(Pos, Pos, String)>,
    ) {
        let before = self.selections.clone();
        let list = before.list().to_vec();
        let primary = before.primary_index();
        let buffer = self.buffer.clone();
        let mut buffer = buffer.borrow_mut();
        let mut planned: Vec<(usize, Pos, Pos, String)> = list
            .iter()
            .enumerate()
            .filter_map(|(index, selection)| {
                edit(buffer.document(), index, selection).map(|(start, end, text)| {
                    let (start, end) = ordered(start, end);
                    (index, start, end, text)
                })
            })
            .collect();
        planned.sort_by_key(|(_, start, _, _)| std::cmp::Reverse(*start));

        let after = buffer.transact(Origin::View(self.id), |tx| {
            tx.set_kind(kind);
            let mut carets: Vec<Option<Pos>> = vec![None; list.len()];
            let mut lowest: Option<Pos> = None;
            for (index, start, end, text) in planned {
                // Ranges overlapping one edited already (two carets deleting
                // into each other) are left alone.
                if lowest.is_some_and(|lowest| end > lowest) {
                    continue;
                }
                if start == end && text.is_empty() {
                    carets[index] = Some(start);
                    continue;
                }
                let new_end = tx.replace(start, end, &text);
                for caret in carets.iter_mut().flatten() {
                    *caret = shift(*caret, end, new_end);
                }
                carets[index] = Some(new_end);
                lowest = Some(start);
            }
            let after = Selections::from_list(
                list.iter()
                    .zip(carets)
                    .map(|(selection, caret)| Selection::caret(caret.unwrap_or(selection.head)))
                    .collect(),
                primary,
            );
            tx.set_selections(before.clone(), after.clone());
            after
        });
        self.selections = after;
        self.selections_seen = buffer.revision();
    }

    fn insert_all(&mut self, text: &str, kind: EditKind) {
        let texts = vec![text.to_owned(); self.selections.len()];
        self.insert_each(texts, kind);
    }

    /// Replaces selection `i` with `texts[i]`.
    fn insert_each(&mut self, texts: Vec<String>, kind: EditKind) {
        self.edit_each(kind, |_, index, selection| {
            Some((selection.start(), selection.end(), texts[index].clone()))
        });
    }

    /// Deletes each selection, or for a caret the range `range` gives.
    fn delete_with(&mut self, kind: EditKind, range: impl Fn(&Document, Pos) -> (Pos, Pos)) {
        self.edit_each(kind, |doc, _, selection| {
            let (start, end) = if selection.is_empty() {
                range(doc, selection.head)
            } else {
                (selection.start(), selection.end())
            };
            Some((start, end, String::new()))
        });
    }

    fn paste(&mut self, text: &str) {
        self.end_composition();
        let text = normalize_breaks(text).into_owned();
        let lines: Vec<&str> = text.split('\n').collect();
        // One line per selection, when the clipboard holds as many lines.
        if self.selections.len() > 1 && lines.len() == self.selections.len() {
            let texts = lines.into_iter().map(str::to_owned).collect();
            self.insert_each(texts, EditKind::Other);
        } else {
            self.insert_all(&text, EditKind::Other);
        }
    }

    /// Tab: at a caret, the indent text; over a selection, every line it
    /// touches is indented.
    fn indent(&mut self) {
        let indent = self.style.indent.clone();
        if self.selections.iter().all(Selection::is_empty) {
            self.insert_all(&indent, EditKind::Typing);
            return;
        }
        self.change_line_starts(|_| Some((0, indent.clone())));
    }

    /// Shift-Tab: up to one indent's worth of leading whitespace off every
    /// line a selection touches. A tab counts as a whole indent.
    fn outdent(&mut self) {
        let width = self.style.indent.len().max(1);
        let removals: FxHashMap<usize, usize> = {
            let buffer = self.buffer.borrow();
            let doc = buffer.document();
            self.lines_touched()
                .into_iter()
                .map(|line| {
                    let mut columns = 0;
                    let mut bytes = 0;
                    for ch in doc.indent(line).chars() {
                        if columns >= width {
                            break;
                        }
                        columns += if ch == '\t' { width } else { 1 };
                        bytes += ch.len_utf8();
                    }
                    (line, bytes)
                })
                .collect()
        };
        self.change_line_starts(|line| {
            removals
                .get(&line)
                .filter(|remove| **remove > 0)
                .map(|remove| (*remove, String::new()))
        });
    }

    /// The lines any selection touches, in order.
    fn lines_touched(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = self
            .selections
            .iter()
            .flat_map(|selection| {
                let (start, end) = (selection.start(), selection.end());
                // A selection ending at a line's start does not take that line.
                let last = if end.byte == 0 && end.line > start.line {
                    end.line - 1
                } else {
                    end.line
                };
                start.line..=last
            })
            .collect();
        lines.dedup();
        lines
    }

    /// Replaces the first `n` bytes of each touched line with `text`, as
    /// `change` says, keeping the selections over the same text.
    fn change_line_starts(&mut self, change: impl Fn(usize) -> Option<(usize, String)>) {
        let before = self.selections.clone();
        let lines = self.lines_touched();
        let buffer = self.buffer.clone();
        let mut buffer = buffer.borrow_mut();
        let after = buffer.transact(Origin::View(self.id), |tx| {
            let mut deltas: FxHashMap<usize, isize> = FxHashMap::default();
            for line in lines.into_iter().rev() {
                let Some((remove, text)) = change(line) else {
                    continue;
                };
                tx.replace(Pos::new(line, 0), Pos::new(line, remove), &text);
                deltas.insert(line, text.len() as isize - remove as isize);
            }
            let shift_pos = |pos: Pos| {
                let delta = deltas.get(&pos.line).copied().unwrap_or(0);
                Pos::new(pos.line, (pos.byte as isize + delta).max(0) as usize)
            };
            let mut after = before.clone();
            after.map(|selection| {
                Selection::new(shift_pos(selection.anchor), shift_pos(selection.head))
            });
            tx.set_selections(before.clone(), after.clone());
            after
        });
        self.selections = after;
        self.selections_seen = buffer.revision();
    }

    fn selected_text(&self) -> String {
        let buffer = self.buffer.borrow();
        let doc = buffer.document();
        self.selections
            .iter()
            .filter(|selection| !selection.is_empty())
            .map(|selection| doc.slice(selection.start(), selection.end()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// An IME event. A composition's text sits in the buffer unrecorded,
    /// replacing the selection, so it lays out and wraps like any other and
    /// other views see it; only the committed text becomes an undo step.
    fn ime(&mut self, ime: &RawIme) -> EventResult {
        if self.read_only {
            return EventResult::Ignored;
        }
        match ime {
            RawIme::Enabled { .. } => {}
            RawIme::Preedit { text, cursor, .. } => {
                let text = text.as_str();
                if self.composition.is_none() {
                    // Composing replaces the selection: delete it as a step
                    // of its own, so undo brings it back.
                    self.selections.collapse();
                    if !self.selections.primary().is_empty() {
                        self.delete_with(EditKind::Other, |_, at| (at, at));
                    }
                }
                let previous = self.composition.take();
                let start = previous.map_or(self.selections.primary().head, |old| old.start);
                let buffer = self.buffer.clone();
                let mut buffer = buffer.borrow_mut();
                let end = buffer.transact(Origin::View(self.id), |tx| {
                    tx.untracked();
                    match previous {
                        Some(old) => tx.replace(old.start, old.end, text),
                        None => tx.insert(start, text),
                    }
                });
                self.selections_seen = buffer.revision();
                drop(buffer);
                let caret = if text.is_empty() {
                    start
                } else {
                    self.composition = Some(Composition { start, end });
                    cursor.map_or(end, |cursor| {
                        let byte = cursor.end.offset().min(text.len());
                        end_of(start, &text[..byte])
                    })
                };
                self.selections = Selections::single(Selection::caret(caret));
            }
            RawIme::Commit { text, .. } => {
                self.end_composition();
                self.insert_all(text.as_str(), EditKind::Typing);
            }
            RawIme::Disabled { .. } => self.end_composition(),
        }
        EventResult::Consumed
    }

    /// Takes a composition's text out again, if one is in progress.
    fn end_composition(&mut self) {
        let Some(composition) = self.composition.take() else {
            return;
        };
        let buffer = self.buffer.clone();
        let mut buffer = buffer.borrow_mut();
        buffer.transact(Origin::View(self.id), |tx| {
            tx.untracked();
            tx.delete(composition.start, composition.end);
        });
        self.selections_seen = buffer.revision();
        self.selections = Selections::single(Selection::caret(composition.start));
    }

    fn surface_position(&self, pos: Pos) -> SurfacePosition {
        self.buffer.read(|buffer| {
            let doc = buffer.document();
            let pos = doc.clamp(pos);
            SurfacePosition::new(doc.id(pos.line), pos.byte)
        })
    }

    fn pos_of(&self, at: SurfacePosition) -> Pos {
        let line = self.buffer.read(|buffer| {
            let doc = buffer.document();
            doc.line_of(at.block)
                .map(|line| doc.clamp(Pos::new(line, at.offset)))
        });
        line.unwrap_or(self.selections.primary().head)
    }
}

impl TextSurfaceModel for EditorView {
    fn take_changes(&mut self) -> SurfaceChanges {
        self.catch_up();
        let mut reset = std::mem::take(&mut self.reset);
        let mut blocks = Vec::new();
        let revision = {
            let buffer = self.buffer.borrow();
            match buffer.log_since(self.blocks_seen) {
                None => reset = true,
                Some(entries) => {
                    for entry in entries {
                        match &entry.change {
                            LogChange::Edit {
                                blocks: changed, ..
                            }
                            | LogChange::Restyle { blocks: changed } => {
                                blocks.extend_from_slice(changed)
                            }
                            LogChange::Reset => reset = true,
                        }
                    }
                }
            }
            buffer.revision()
        };
        self.blocks_seen = revision;
        self.buffer
            .borrow_mut()
            .mark_seen(self.id, self.blocks_seen.min(self.selections_seen));
        if reset {
            SurfaceChanges::Reset
        } else if blocks.is_empty() {
            SurfaceChanges::None
        } else {
            SurfaceChanges::Edits(blocks)
        }
    }

    fn block_count(&self) -> usize {
        self.buffer.read(|buffer| buffer.document().line_count())
    }

    fn block_at(&self, index: usize) -> BlockId {
        self.buffer.read(|buffer| buffer.document().id(index))
    }

    fn block(&self, id: BlockId) -> SurfaceBlock {
        let Some((text, runs)) = self.buffer.read(|buffer| buffer.styled_line(id)) else {
            return SurfaceBlock::default();
        };
        SurfaceBlock {
            text,
            runs,
            paragraph: ParagraphStyle {
                white_space: if self.style.wrap {
                    WhiteSpace::PreWrap
                } else {
                    WhiteSpace::Pre
                },
                ..ParagraphStyle::default()
            },
        }
    }

    fn decorations(&self, _visible: &[BlockId]) -> Vec<SurfaceDecor> {
        let mut decorations = Vec::new();
        for selection in self.selections.iter() {
            if !selection.is_empty() {
                decorations.push(SurfaceDecor {
                    range: SurfaceRange::new(
                        self.surface_position(selection.start()),
                        self.surface_position(selection.end()),
                    ),
                    kind: DecorKind::Background(self.style.selection),
                    layer: DecorLayer::BelowText,
                });
            }
            decorations.push(SurfaceDecor {
                range: SurfaceRange::caret(self.surface_position(selection.head)),
                kind: DecorKind::Caret {
                    color: self.style.caret,
                    width: self.style.caret_width,
                },
                layer: DecorLayer::AboveText,
            });
        }
        if let Some(composition) = self.composition {
            decorations.push(SurfaceDecor {
                range: SurfaceRange::new(
                    self.surface_position(composition.start),
                    self.surface_position(composition.end),
                ),
                kind: DecorKind::Underline {
                    color: self.style.composition,
                    style: UnderlineStyle::Solid,
                },
                layer: DecorLayer::AboveText,
            });
        }
        decorations
    }

    fn primary_caret(&self) -> Option<SurfacePosition> {
        Some(self.surface_position(self.selections.primary().head))
    }

    fn handle_input(&mut self, input: SurfaceInput<'_>, cx: &mut SurfaceCx<'_, '_>) -> EventResult {
        self.handle(input, cx)
    }
}

/// The editor command a standard command id means.
fn command_for(command: CommandId) -> Option<EditorCommand> {
    Some(match command {
        c if c == commands::COPY => EditorCommand::Copy,
        c if c == commands::CUT => EditorCommand::Cut,
        c if c == commands::PASTE => EditorCommand::Paste,
        c if c == commands::SELECT_ALL => EditorCommand::SelectAll,
        c if c == commands::UNDO => EditorCommand::Undo,
        c if c == commands::REDO => EditorCommand::Redo,
        _ => return None,
    })
}

/// Home: to the first non-blank of the line, or to its start from there.
fn line_start(doc: &Document, head: Pos) -> Pos {
    let indent = doc.indent(head.line).len();
    if head.byte == indent {
        Pos::new(head.line, 0)
    } else {
        Pos::new(head.line, indent)
    }
}

#[cfg(test)]
mod tests;
